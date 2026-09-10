//! The whole grant matrix, on the layer that would actually ship.
//!
//! WHY THIS SPIKE EXISTS. `spike/p2panda-seal` drove `data_scheme::EncryptionGroup`
//! directly, filling its two hardest generic slots — group membership and message
//! ordering — with the crate's `test_utils`. It measured that a reader added by
//! `add()` receives the history that exists at that moment and then never
//! advances, and it was careful to say the root cause was not established.
//!
//! It is not established here either, because it does not need to be:
//! **`p2panda-spaces` ships the real implementations of both slots** and its own
//! test suite asserts that a member added to a space decrypts what is published
//! afterwards. So the question is no longer "does `add` work" but "what does the
//! shipping layer give us, and what is left to write ourselves".
//!
//! FOUR QUESTIONS, which together are every grant this project offers:
//!
//!   1. Does a reader added to a space open what was published BEFORE the add?
//!      That is "grant with history", and if it is yes it is free.
//!   2. Does it open what is published AFTER? That is an ordinary grant.
//!   3. Does a reader in one space open another space's data? If no, then
//!      "one space per grant window" gives "from now on" with no cryptography
//!      of our own — the reader is simply not in the earlier space.
//!   4. Does `remove` cut access to what comes next, and leave what they had?
//!
//! `Space` has no `update()`, so a key is not rotated per epoch the way
//! `tools/seal.py` does it. Rotation happens on membership change. That is a
//! real difference from the reference construction and question 4 is where it
//! shows up.
//!
//! Run: cargo run --manifest-path spike/p2panda-spaces/Cargo.toml

use futures::FutureExt;
use p2panda_auth::Access;
use p2panda_spaces::SpaceId;
use p2panda_spaces::Event;
use p2panda_spaces::test_utils::{TestOperation, TestPeer};

/// Hand one peer another's messages, in order, and collect what it could read.
///
/// Every message is persisted and processed whether or not this peer can open
/// it — which is the property the whole project rests on: a peer holds
/// ciphertext it cannot read. A message that yields no `Application` event is
/// not an error here, it is the access control working.
async fn feed(who: &str, peer: &TestPeer, msgs: &[TestOperation]) -> Vec<String> {
    
    let mut read = Vec::new();
    for (i, m) in msgs.iter().enumerate() {
        peer.persist_operation(m).await.unwrap();

        // PER MESSAGE, and panic-safe. p2panda-auth panics rather than erroring
        // when it is handed an operation it cannot place, and a spike that dies
        // on the first one measures nothing. Which message kills it is the
        // interesting part.
        let fut = std::panic::AssertUnwindSafe(peer.manager.process_persisted(m));
        match fut.catch_unwind().await {
            Ok(Ok(events)) => {
                for e in events {
                    if let Event::Application { data, .. } = e {
                        read.push(String::from_utf8_lossy(&data).to_string());
                    }
                }
            }
            Ok(Err(e)) => println!("      {who} refused message {i}: {e}"),
            Err(_) => println!("      {who} PANICKED on message {i} of {}", msgs.len()),
        }
    }
    read
}

fn report(label: &str, pass: bool, detail: &str) {
    println!("    {:<46} {}   {}", label, if pass { "PASS" } else { "FAIL" }, detail);
}

#[tokio::main]
async fn main() {
    std::panic::set_hook(Box::new(|_| {}));

    let subject = TestPeer::new(0).await;
    let bob = TestPeer::new(1).await;
    let alice = TestPeer::new(2).await;

    let bob_id = bob.manager.id();
    let alice_id = alice.manager.id();

    // Everyone's key bundle has to be known before they can be added. On a
    // phone this is what the invite QR code carries.
    for (a, b) in [(&subject, &bob), (&subject, &alice), (&bob, &subject), (&alice, &subject)] {
        a.manager.register_member(&b.manager.me().await.unwrap()).await.unwrap();
    }

    println!("\n  p2panda-spaces 0.7.1 — real DGM, real orderer, SQLite store\n");

    // ================= WINDOW A: the subject's first stretch ==============
    let window_a = SpaceId::digest(b"window-a");
    let (space_a, create_a) = subject.manager.create_space_persisted(window_a, &[]).await.unwrap();

    // ONE LOG, IN THE ORDER THE SUBJECT PRODUCED IT.
    //
    // Keeping a vector per space and concatenating them is the obvious way to
    // write this spike and it is wrong: auth messages from the two spaces
    // interleave, and a reader handed them grouped by space sees a space
    // message before the auth message it depends on. That is a real p2panda
    // error — "maybe it arrived out-of-order" — but it was the harness's fault,
    // not the library's. A subject has one log; log sync delivers it in order.
    let mut log: Vec<TestOperation> = create_a;
    for day in 0..2 {
        log.push(space_a.publish_persisted(format!("A day {day}").as_bytes()).await.unwrap());
    }

    // --- bob is granted, AFTER those two days were already published ------
    let (m1, m2) = space_a.add_persisted(bob_id, Access::read()).await.unwrap();
    log.push(m1);
    log.push(m2);

    for day in 2..4 {
        log.push(space_a.publish_persisted(format!("A day {day}").as_bytes()).await.unwrap());
    }

    // --- bob is revoked, and the subject keeps publishing -----------------
    let (m1, m2) = space_a.remove_persisted(bob_id).await.unwrap();
    log.push(m1);
    log.push(m2);

    for day in 4..6 {
        log.push(space_a.publish_persisted(format!("A day {day}").as_bytes()).await.unwrap());
    }

    // ================= WINDOW B: a second space, alice only ===============
    // This is "grant from now on": alice is added to a space that did not
    // exist when window A's days were published, so there is nothing to
    // withhold from her — she was never a member of the space holding them.
    let window_b = SpaceId::digest(b"window-b");
    let (space_b, create_b) = subject.manager.create_space_persisted(window_b, &[]).await.unwrap();

    // MAKE EVERY OTHER SPACE AWARE OF THIS. `p2panda-spaces`' own
    // `shared_auth_state` test does exactly this after every auth-level change,
    // with the comment "Make Space 0 aware of this change". Spaces share one
    // global auth state, so a space that has not been repaired after another
    // one changed it is working from a stale view of that state.
    let repair: Vec<TestOperation> =
        subject.manager.repair_spaces_persisted(&[window_a]).await.unwrap();
    println!("    repaired window A after window B was created: {} message(s)", repair.len());

    log.extend(create_b);
    log.extend(repair);
    let (m1, m2) = space_b.add_persisted(alice_id, Access::read()).await.unwrap();
    log.push(m1);
    log.push(m2);
    for day in 6..8 {
        log.push(space_b.publish_persisted(format!("B day {day}").as_bytes()).await.unwrap());
    }

    // ================= what each reader can actually open =================
    //
    // Each reader is fed only the space it was granted. Feeding a reader the
    // OTHER space's messages is a separate experiment below, because it does
    // not merely fail — it panics, which is a finding of its own.
    let bob_read = feed("bob", &bob, &log).await;

    let alice_read = feed("alice", &alice, &log).await;

    println!("    bob   (added mid-window A, then revoked) opens {bob_read:?}");
    println!("    alice (added to window B only)           opens {alice_read:?}");

    println!("\n  the grant matrix\n");
    report(
        "1. an added reader opens EARLIER days",
        bob_read.iter().any(|d| d == "A day 0"),
        &format!("'A day 0' was published before bob was added — {}",
            if bob_read.iter().any(|d| d == "A day 0") { "he can read it" } else { "he cannot" }),
    );
    report(
        "2. an added reader opens later days",
        bob_read.iter().any(|d| d == "A day 2") && bob_read.iter().any(|d| d == "A day 3"),
        "days 2 and 3, published after the add",
    );
    report(
        "3. a space is a window — no cross-space reading",
        !alice_read.iter().any(|d| d.starts_with("A ")) && alice_read.iter().any(|d| d.starts_with("B ")),
        &format!("alice, a member of B only, opens {alice_read:?}"),
    );
    report(
        "4. remove cuts what comes next",
        !bob_read.iter().any(|d| d == "A day 4" || d == "A day 5"),
        "days 4 and 5, published after the removal",
    );

    // ================= a space you are not in ============================
    //
    // THE CASE THIS PROJECT IS BUILT ON. A diaswarm peer holds ciphertext for
    // strangers, so it will be handed messages for spaces it is not a member
    // of, constantly. What does the library do when that happens?
    println!("\n  a peer handed a space it is not in\n");
    let stranger = TestPeer::new(3).await;
    stranger
        .manager
        .register_member(&subject.manager.me().await.unwrap())
        .await
        .unwrap();
    let stranger_read = feed("stranger", &stranger, &log).await;
    report(
        "5. a stranger processes everything, reads nothing",
        stranger_read.is_empty(),
        &format!("read {stranger_read:?}"),
    );

    // ================= a reader in TWO of one subject's spaces ============
    //
    // Nothing above tests this, and the whole "one space per grant window"
    // design depends on it: a reader granted with history is a member of every
    // window, not just one.
    println!("\n  a reader in two of one subject's spaces\n");
    let both = TestPeer::new(4).await;
    both.manager.register_member(&subject.manager.me().await.unwrap()).await.unwrap();
    subject.manager.register_member(&both.manager.me().await.unwrap()).await.unwrap();
    let both_id = both.manager.id();

    
    // ASK WHICH SPACES ARE STALE, AND FIX THEM FIRST. Every auth-level change
    // in any space — including alice being added to B — leaves the others
    // working from an old view of the shared auth state. The manager will say
    // which ones need it rather than us guessing.
    let needs = subject.manager.spaces_repair_required().await.unwrap();
    println!("      spaces needing repair before this: {}", needs.len());
    if !needs.is_empty() {
        let fixes = subject.manager.repair_spaces_persisted(&needs).await.unwrap();
        println!("      repair produced {} message(s)", fixes.len());
        log.extend(fixes);
    }

    let added_a = std::panic::AssertUnwindSafe(space_a.add_persisted(both_id, Access::read()))
        .catch_unwind()
        .await;
    match added_a {
        Ok(Ok((m1, m2))) => {
            log.push(m1);
            log.push(m2);
        }
        Ok(Err(e)) => println!("      adding to space A refused: {e}"),
        Err(_) => println!("      adding to space A PANICKED (subject side)"),
    }
    // AND AGAIN. Adding to A was itself an auth change, so B is now the stale
    // one. Repair is not a one-off before a batch of work — it belongs before
    // every single auth-level operation.
    let needs = subject.manager.spaces_repair_required().await.unwrap();
    println!("      spaces needing repair between the two adds: {}", needs.len());
    if !needs.is_empty() {
        let fixes = subject.manager.repair_spaces_persisted(&needs).await.unwrap();
        log.extend(fixes);
    }

    let added_b = std::panic::AssertUnwindSafe(space_b.add_persisted(both_id, Access::read()))
        .catch_unwind()
        .await;
    match added_b {
        Ok(Ok((m1, m2))) => {
            log.push(m1);
            log.push(m2);
        }
        Ok(Err(e)) => println!("      adding to space B refused: {e}"),
        Err(_) => println!("      adding to space B PANICKED (subject side)"),
    }
    if let Ok(m) = space_a.publish_persisted(b"A after both").await {
        log.push(m);
    }
    if let Ok(m) = space_b.publish_persisted(b"B after both").await {
        log.push(m);
    }

    let both_read = feed("both", &both, &log).await;
    report(
        "6. a reader can belong to two of one subject's spaces",
        both_read.iter().any(|d| d == "A after both") && both_read.iter().any(|d| d == "B after both"),
        &format!("opens {both_read:?}"),
    );

    // ================= FAN-OUT: a day published into every live window ====
    //
    // §6 says a reader can be in exactly one space. That does not sink the
    // window design, it reshapes it: instead of carrying readers forward into
    // each new window, every reader stays in the ONE window they were granted
    // in, and the subject publishes each day into every window that is still
    // live. Nobody is ever in two spaces, and a reader granted "from now on"
    // still cannot see anything sealed before their window existed.
    //
    // The cost is duplicated ciphertext — one copy per live window — and that
    // is the trade this measures rather than assumes.
    println!("\n  fan-out: publishing into every live window\n");
    let mut fan: Vec<TestOperation> = Vec::new();
    for day in 10..12 {
        fan.push(space_a.publish_persisted(format!("A day {day}").as_bytes()).await.unwrap());
        fan.push(space_b.publish_persisted(format!("B day {day}").as_bytes()).await.unwrap());
    }
    log.extend(fan);

    let bob_after = feed("bob", &bob, &log).await;
    let alice_after = feed("alice", &alice, &log).await;
    report(
        "7. both windows keep receiving, each to its own reader",
        alice_after.iter().any(|d| d == "B day 11") && !alice_after.iter().any(|d| d.starts_with("A ")),
        &format!("alice now opens {alice_after:?}"),
    );
    let _ = bob_after;

    println!("\n  what this means for the design");
    let history_free = bob_read.iter().any(|d| d == "A day 0");
    let windows_work = !alice_read.iter().any(|d| d.starts_with("A "));
    println!(
        "    grant WITH HISTORY   {}",
        if history_free { "space.add() — free" } else { "NOT available from add()" }
    );
    println!(
        "    grant FROM NOW ON    {}",
        if windows_work { "a new space — free" } else { "NOT expressible as a space" }
    );
    println!(
        "    revocation           {}",
        if !bob_read.iter().any(|d| d == "A day 4") { "space.remove() — free" } else { "LEAKS" }
    );
}
