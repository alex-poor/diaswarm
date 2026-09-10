//! Can a *group* inside one space do what a second space was supposed to do?
//!
//! WHY THIS MATTERS MORE THAN IT LOOKS. `FINDINGS.md §5` measured that a subject
//! cannot have two spaces — adding a member to the first one, once a second
//! exists, panics `p2panda-auth` on the subject's side. That blocks "grant from
//! now on", which is expressed as a second space.
//!
//! Before calling that a defect, the question is whether two spaces is even the
//! intended way to say it. `p2panda-spaces` also has groups: `create_group`
//! returns an id that is itself an `ActorId`, so a group can be added to a space
//! as though it were a person, and people can be added to the group afterwards.
//! If a group's members only get secrets from the point they join it, then a
//! window is a group, everything happens inside ONE space, and nothing upstream
//! needs to change.
//!
//! THE DECIDING QUESTION: a group is in a space, a day is published, and THEN
//! somebody joins the group. Can they read the day that was published before
//! they joined?
//!
//!   * **no**  → groups scope history; windows live inside one space; unblocked.
//!   * **yes** → groups are authorisation only, not key custody, and separate
//!               spaces really are the only way to express a window.
//!
//! The documentation points at "yes": `space.rs` calls a space "a single
//! encryption context", and the only access level excluded from that context is
//! `Pull`, which is permission to sync rather than to read. But the same
//! documentation is what made `add()` look all-or-nothing before it was
//! measured, so this is measured.
//!
//! ANSWERED, AND NOT THE WAY EITHER BRANCH BELOW EXPECTS. A group cannot be
//! added to a space as a read member at all: the encryption context needs a
//! prekey bundle per member and a group id is not a key holder, so
//! `space.add(group.id(), Access::read())` fails with `MissingPreKeys` for the
//! group's own key. The library's `shared_auth_state` test does add a group to
//! a space with read access — after creating the group WITH members, and after
//! repairing every other space. Chasing that is what turned up the repair
//! discipline in `FINDINGS.md` §5a, which is what actually mattered.
//!
//! Kept as it stands because the failure it produces is the useful part, and
//! because `FINDINGS.md` §5c found the answer elsewhere: a window is a space
//! with one cohort in it, not a group.
//!
//! Run: cargo run --manifest-path spike/p2panda-spaces/Cargo.toml --bin groups

use futures::FutureExt;
use p2panda_auth::Access;
use p2panda_spaces::test_utils::{TestOperation, TestPeer};
use p2panda_spaces::{Event, SpaceId};

async fn feed(who: &str, peer: &TestPeer, msgs: &[TestOperation]) -> Vec<String> {
    let mut read = Vec::new();
    for (i, m) in msgs.iter().enumerate() {
        peer.persist_operation(m).await.unwrap();
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
            Err(_) => println!("      {who} PANICKED on message {i}"),
        }
    }
    read
}

#[tokio::main]
async fn main() {
    let subject = TestPeer::new(0).await;
    let early = TestPeer::new(1).await;
    let late = TestPeer::new(2).await;

    for (a, b) in [
        (&subject, &early),
        (&subject, &late),
        (&early, &subject),
        (&late, &subject),
    ] {
        a.manager.register_member(&b.manager.me().await.unwrap()).await.unwrap();
    }

    println!("\n  p2panda-spaces 0.7.1 — a group inside one space\n");

    // A group, and one space that the group is a member of.
    let (group, group_msg) =
        subject.manager.create_group_persisted(&[]).await.unwrap();
    let mut msgs: Vec<TestOperation> = vec![group_msg];

    let space_id = SpaceId::digest(b"one-space");
    let (space, create) = subject.manager.create_space_persisted(space_id, &[]).await.unwrap();
    msgs.extend(create);

    // The group joins the space as though it were a person.
    let (m1, m2) = space.add_persisted(group.id(), Access::read()).await.unwrap();
    msgs.push(m1);
    msgs.push(m2);
    println!("    group {} added to the space with read access", &group.id().to_hex()[..16]);

    // EARLY joins the group, then a day is published.
    msgs.push(group.add_persisted(early.manager.id(), Access::read()).await.unwrap());
    msgs.push(space.publish_persisted(b"day 0").await.unwrap());
    println!("    early joined the group, then 'day 0' was published");

    // LATE joins the group AFTER day 0 exists, then day 1 is published.
    msgs.push(group.add_persisted(late.manager.id(), Access::read()).await.unwrap());
    msgs.push(space.publish_persisted(b"day 1").await.unwrap());
    println!("    late joined the group, then 'day 1' was published\n");

    let early_read = feed("early", &early, &msgs).await;
    let late_read = feed("late", &late, &msgs).await;

    println!("\n    early opens {early_read:?}");
    println!("    late  opens {late_read:?}");

    let late_sees_day_0 = late_read.iter().any(|d| d == "day 0");
    let late_sees_day_1 = late_read.iter().any(|d| d == "day 1");

    println!("\n  the answer\n");
    if !late_sees_day_1 {
        println!("    INCONCLUSIVE — a group member read nothing at all, so this says");
        println!("    nothing about history. Group membership may not reach the");
        println!("    encryption context in this version.");
    } else if late_sees_day_0 {
        println!("    GROUPS DO NOT SCOPE HISTORY.");
        println!("    Somebody who joined the group after 'day 0' was published can");
        println!("    read it. A group is authorisation, not key custody — the space");
        println!("    is one encryption context and joining it hands over the whole");
        println!("    secret bundle, exactly as `add()` on a space does.");
        println!("    A window therefore cannot be a group, and §5's blocker stands.");
    } else {
        println!("    GROUPS SCOPE HISTORY — §5's blocker is avoidable.");
        println!("    A window is a group inside one space: everything stays in a");
        println!("    single space, so the second-space panic is never reached, and");
        println!("    'grant from now on' needs nothing from upstream.");
    }
}
