//! Can "include my history / from now on" be a choice per grant, using only the library?
//!
//! WHY THIS DECIDES THE ARCHITECTURE. The hand-rolled construction gets history
//! scoping for free: keys are wrapped per reader per segment, so a grant simply
//! never reaches backwards. `FINDINGS.md §3` measured that p2panda's `add()`
//! cannot do that — it welcomes a joiner with the entire `SecretBundle`, so a
//! reader added today opens every epoch ever sealed. Its conclusion was that
//! time-scoping "has to come from group partitioning", and left that untested.
//!
//! This is the test. If it holds, the choice can be offered per grant with no
//! bespoke cryptography at all: **a window is a group**.
//!
//!   * sealing happens in the CURRENT window's group;
//!   * granting *from now on* starts a new window, and the reader joins only
//!     that one — every earlier window's secrets are simply never sent to them;
//!   * granting *with history* adds the reader to every window that exists;
//!   * a new window takes all current readers with it, so nobody loses access
//!     by someone else being granted;
//!   * revoking removes from the current window, which p2panda rotates
//!     immediately.
//!
//! The cost to measure alongside the property: how many group instances and
//! control messages this actually takes, because that is what a phone pays.
//!
//! Run: cargo run --manifest-path spike/p2panda-seal/Cargo.toml --bin windows

use std::collections::HashMap;

use p2panda_encryption::crypto::Rng;
use p2panda_encryption::crypto::x25519::SecretKey;
use p2panda_encryption::data_scheme::{
    EncryptionGroup, GroupOutput, GroupState, decrypt_data,
};
use p2panda_encryption::key_bundle::Lifetime;
use p2panda_encryption::key_manager::{KeyManager, KeyManagerState};
use p2panda_encryption::key_registry::{KeyRegistry, KeyRegistryState};
use p2panda_encryption::test_utils::data_scheme::dgm::TestDgm;
use p2panda_encryption::test_utils::data_scheme::ordering::{MessageOrderer, TestMessage};
use p2panda_encryption::test_utils::{MemberId, MessageId};
use p2panda_encryption::traits::{GroupMessage, GroupMessageContent, PreKeyManager};

type Dgm = TestDgm<MemberId, MessageId>;
type Ord_ = MessageOrderer<Dgm>;
type Group = EncryptionGroup<MemberId, MessageId, KeyRegistry<MemberId>, Dgm, KeyManager, Ord_>;
type State = GroupState<MemberId, MessageId, KeyRegistry<MemberId>, Dgm, KeyManager, Ord_>;

const SUBJECT: MemberId = 0;
const BOB: MemberId = 1;
const ALICE: MemberId = 2;
const CAROL: MemberId = 3;
const NAMES: [&str; 4] = ["subject", "bob", "alice", "carol"];

struct Party {
    id: MemberId,
    km: KeyManagerState,
    /// One group state per window this party belongs to. A reader granted
    /// "from now on" simply has no entry for the earlier windows, and that
    /// absence IS the access control — there is nothing to withhold.
    windows: HashMap<usize, State>,
}

/// Control-message delivery within one window.
///
/// Scoped per window because a party's state is per window: the same person is
/// a different member of a different group in each one, and delivering a
/// window's welcome into another window's state is how this construction would
/// quietly collapse into the all-or-nothing behaviour it exists to avoid.
fn deliver(
    parties: &mut [Party],
    logs: &mut HashMap<usize, Vec<TestMessage<Dgm>>>,
    w: usize,
    msgs: Vec<TestMessage<Dgm>>,
    errors: &mut usize,
) -> usize {
    let mut delivered = 0;
    let mut pending = msgs;
    while let Some(msg) = pending.pop() {
        delivered += 1;
        // EVERY CONTROL MESSAGE IS KEPT. A window's control messages are its
        // log; a reader granted history later has to be able to replay them,
        // and the first version of this spike delivered each message once to
        // whoever existed at the time. A reader added afterwards then had a
        // welcome it could not order, opened nothing, and that looked exactly
        // like the library refusing to grant history.
        logs.entry(w).or_default().push(msg.clone());

        for p in parties.iter_mut() {
            // Feeding the sender its own control message was tried: it changes
            // nothing here and the library refuses it, so a sender's own state
            // is already up to date when the call returns.
            if msg.sender() == p.id {
                continue;
            }
            // Clone rather than remove: a removed member must keep the state it
            // had, or the thing being measured disappears with it.
            let Some(state) = p.windows.get(&w).cloned() else { continue };
            match Group::receive(state, &msg) {
                Ok((next, out)) => {
                    p.windows.insert(w, next);
                    for o in out {
                        if let GroupOutput::Control(m) = o {
                            pending.push(m);
                        }
                    }
                }
                // Counted, not swallowed. A silently dropped control message is
                // indistinguishable from access being correctly withheld.
                Err(_) => *errors += 1,
            }
        }
    }
    delivered
}

/// Bring a party up to date on a window it has just joined.
///
/// This is the sync a real reader does: the window's control messages are in a
/// log it replicates, and it has to process them in order before the welcome
/// addressed to it means anything.
fn catch_up(
    p: &mut Party,
    logs: &HashMap<usize, Vec<TestMessage<Dgm>>>,
    w: usize,
    errors: &mut usize,
) {
    let Some(log) = logs.get(&w) else { return };
    for msg in log {
        if msg.sender() == p.id {
            continue;
        }
        let Some(state) = p.windows.get(&w).cloned() else { return };
        match Group::receive(state, msg) {
            Ok((next, _)) => {
                p.windows.insert(w, next);
            }
            Err(_) => *errors += 1,
        }
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let rng = Rng::from_seed([11; 32]);

    let mut kms = Vec::new();
    let mut bundles = Vec::new();
    for _ in 0..NAMES.len() {
        let secret = SecretKey::from_bytes(rng.random_array()?);
        let km = KeyManager::init_and_generate_prekey(&secret, Lifetime::default(), &rng)?;
        bundles.push(KeyManager::prekey_bundle(&km)?);
        kms.push(km);
    }
    let registry = {
        let mut pki = KeyRegistry::init();
        for (id, b) in bundles.iter().enumerate() {
            pki = KeyRegistry::add_longterm_bundle(pki, id, b.clone())?;
        }
        pki
    };

    let mut parties: Vec<Party> = (0..NAMES.len())
        .map(|id| Party { id, km: kms[id].clone(), windows: HashMap::new() })
        .collect();

    // A party joining a window initialises its state for that window, exactly
    // as a phone would on receiving the welcome.
    fn init_in(p: &mut Party, w: usize, registry: &KeyRegistryState<MemberId>) {
        p.windows.entry(w).or_insert_with(|| {
            Group::init(
                p.id,
                p.km.clone(),
                registry.clone(),
                TestDgm::init(p.id),
                MessageOrderer::init(p.id),
            )
        });
    }

    println!("\n  p2panda-encryption 0.7.1 — one group per grant window\n");

    let mut control = 0usize;
    let mut current = 0usize;
    let mut errors = 0usize;
    let mut logs: HashMap<usize, Vec<TestMessage<Dgm>>> = HashMap::new();

    // --- window 0: nobody granted yet, the subject seals for itself --------
    init_in(&mut parties[SUBJECT], 0, &registry);
    let (state, welcome) = Group::create(parties[SUBJECT].windows.remove(&0).unwrap(), vec![], &rng)?;
    parties[SUBJECT].windows.insert(0, state);
    control += deliver(&mut parties, &mut logs, 0, vec![welcome], &mut errors);

    // sealed[epoch] = (window, message)
    let mut sealed: Vec<(usize, usize, TestMessage<Dgm>)> = Vec::new();

    for epoch in 0..10usize {
        // --- grants and revocations land at the start of an epoch ----------
        match epoch {
            3 => {
                // BOB, from now on. A NEW WINDOW, and he is its only reader.
                let w = current + 1;
                init_in(&mut parties[SUBJECT], w, &registry);
                init_in(&mut parties[BOB], w, &registry);
                let (state, msg) =
                    Group::create(parties[SUBJECT].windows.remove(&w).unwrap(), vec![BOB], &rng)?;
                parties[SUBJECT].windows.insert(w, state);
                control += deliver(&mut parties, &mut logs, w, vec![msg], &mut errors);
                current = w;
                println!("    epoch 3  grant bob   FROM NOW ON      → window {w} opened");
            }
            5 => {
                // ALICE, with history. Added to EVERY window that exists.
                for w in 0..=current {
                    init_in(&mut parties[ALICE], w, &registry);
                    catch_up(&mut parties[ALICE], &logs, w, &mut errors);
                    let (state, msg) =
                        Group::add(parties[SUBJECT].windows.remove(&w).unwrap(), ALICE, &rng)?;
                    parties[SUBJECT].windows.insert(w, state);
                    control += deliver(&mut parties, &mut logs, w, vec![msg], &mut errors);
                }
                println!(
                    "    epoch 5  grant alice WITH HISTORY     → added to windows 0..={current}"
                );
            }
            7 => {
                // CAROL, from now on. New window, and every current reader
                // comes with it so nobody loses access.
                let w = current + 1;
                init_in(&mut parties[SUBJECT], w, &registry);
                let mut members = vec![BOB, ALICE, CAROL];
                members.sort();
                for m in &members {
                    init_in(&mut parties[*m], w, &registry);
                }
                let (state, msg) =
                    Group::create(parties[SUBJECT].windows.remove(&w).unwrap(), members, &rng)?;
                parties[SUBJECT].windows.insert(w, state);
                control += deliver(&mut parties, &mut logs, w, vec![msg], &mut errors);
                current = w;
                println!("    epoch 7  grant carol FROM NOW ON      → window {w} opened");
            }
            8 => {
                let (state, msg) =
                    Group::remove(parties[SUBJECT].windows.remove(&current).unwrap(), BOB, &rng)?;
                parties[SUBJECT].windows.insert(current, state);
                control += deliver(&mut parties, &mut logs, current, vec![msg], &mut errors);
                println!("    epoch 8  revoke bob                   → removed from window {current}");
            }
            _ => {}
        }

        // --- rotate the current window's key, then seal the day ------------
        if epoch > 0 {
            let (state, msg) =
                Group::update(parties[SUBJECT].windows.remove(&current).unwrap(), &rng)?;
            parties[SUBJECT].windows.insert(current, state);
            control += deliver(&mut parties, &mut logs, current, vec![msg], &mut errors);
        }

        let day = format!("epoch {epoch}: cgm 100.{epoch}");
        let (state, msg) =
            Group::send(parties[SUBJECT].windows.remove(&current).unwrap(), day.as_bytes(), &rng)?;
        parties[SUBJECT].windows.insert(current, state);
        sealed.push((epoch, current, msg.clone()));

        // Everyone holds the ciphertext, readable or not.
        for p in parties.iter_mut().filter(|p| p.id != SUBJECT) {
            if let Some(state) = p.windows.get(&current).cloned() {
                if let Ok((next, _)) = Group::receive(state, &msg) {
                    p.windows.insert(current, next);
                }
            }
        }
    }

    // --- what each reader can open, from the keys it holds -----------------
    //
    // Asked against held keys and held bytes, not by replaying through
    // receive(): the orderer refuses a message it has already seen, so a replay
    // reports nothing for everybody and looks like a revocation that worked.
    println!();

    // WHAT A READER CAN OPEN, ASKED THE WAY A READER ACTUALLY ASKS IT.
    //
    // A fresh group state per window, with that window's whole control log
    // replayed into it in order, and then the sealed days tried against the
    // keys that state ended up holding. This is what a follower does: it syncs
    // the log and processes it — it does not sit online for the group's entire
    // history.
    //
    // The live states carried through the loop above are NOT the measurement.
    // Delivering each message once, to whoever existed at that moment, left a
    // reader added later holding the history bundle and then never advancing —
    // which read exactly like the library refusing to keep granting, and is
    // instead an artefact of a harness that has no log.
    //
    // Replay is also the honest test of revocation: a removed reader replays
    // the same log as everyone else, including every message sent after it was
    // removed, and must still open nothing new. Secrets it was not encrypted to
    // are not in the log in any form it can use.
    let mut opens: HashMap<MemberId, Vec<usize>> = HashMap::new();
    let mut replay_errors = 0usize;
    for id in [BOB, ALICE, CAROL] {
        let mut can = Vec::new();
        let mut held = Vec::new();
        let mut ws: Vec<usize> = parties[id].windows.keys().copied().collect();
        ws.sort();

        for w in &ws {
            let mut state = Group::init(
                id,
                parties[id].km.clone(),
                registry.clone(),
                TestDgm::init(id),
                MessageOrderer::init(id),
            );
            for msg in logs.get(w).into_iter().flatten() {
                if msg.sender() == id {
                    continue;
                }
                match Group::receive(state.clone(), msg) {
                    Ok((next, _)) => state = next,
                    Err(_) => replay_errors += 1,
                }
            }
            held.push(format!("w{w}:{}", state.secrets.len()));

            for (epoch, sealed_in, msg) in &sealed {
                if sealed_in != w {
                    continue;
                }
                let GroupMessageContent::Application { ciphertext, group_secret_id, nonce } =
                    msg.content()
                else {
                    continue;
                };
                if let Some(secret) = state.secrets.get(&group_secret_id) {
                    if decrypt_data(&ciphertext, secret, nonce).is_ok() {
                        can.push(*epoch);
                    }
                }
            }
        }
        can.sort();
        println!(
            "    {:<7} in windows {:?}  secrets {}  opens epochs {:?}",
            NAMES[id],
            ws,
            held.join(" "),
            can
        );
        opens.insert(id, can);
    }

    let bob = &opens[&BOB];
    let alice = &opens[&ALICE];
    let carol = &opens[&CAROL];

    let report = |label: &str, pass: bool, detail: String| {
        println!("    {:<44} {}   {}", label, if pass { "PASS" } else { "FAIL" }, detail);
    };

    println!("\n  the property\n");
    report(
        "'from now on' withholds earlier epochs",
        !bob.iter().any(|e| *e < 3) && !carol.iter().any(|e| *e < 7),
        format!("bob {bob:?}, carol {carol:?}"),
    );
    report(
        "'with history' opens everything",
        (0..10).all(|e| alice.contains(&e)),
        format!("alice opens {} of 10", alice.len()),
    );
    report(
        "the choice is per grant, same subject",
        !bob.contains(&0) && alice.contains(&0),
        "bob withheld epoch 0, alice granted it".to_string(),
    );
    report(
        "a new window does not cut existing readers",
        bob.contains(&7) && alice.contains(&7),
        format!("bob and alice both opened epoch 7 in the new window"),
    );
    report(
        "revocation still cuts immediately",
        !bob.iter().any(|e| *e >= 8) && carol.contains(&8) && carol.contains(&9),
        format!("bob stops at {:?}, carol continues", bob.iter().max()),
    );

    println!("\n  the cost");
    println!("    control messages refused  {errors} live, {replay_errors} on replay");
    println!("    windows opened            {}", current + 1);
    println!("    control messages          {control}");
    println!(
        "    group states on the phone {} for the subject, {} for a with-history reader",
        parties[SUBJECT].windows.len(),
        parties[ALICE].windows.len()
    );

    Ok(())
}
