//! Can a follower still FIND a subject when something moves?
//!
//! **EVERY OTHER TEST IN THIS CRATE HANDS OVER AN ADDRESS.** They follow with
//! `<node id>@<ip>:<port>`, taken from the subject's live endpoint moments
//! earlier, and then assert that bytes arrive. That proves fetching. It cannot
//! prove *discovery*, because nothing ever has to be discovered — and a
//! follower that can only reach a subject at the address it was handed works
//! perfectly until the subject's phone changes address, which happens every
//! night.
//!
//! On 2026-09-14 that gap cost a full overnight outage: the loop phone slept,
//! came back on a different address, and the follower went blind and stayed
//! blind through three restarts. ~125 tests were green throughout. These are
//! the scenarios that were missing, written so that a regression in transport
//! fails here rather than on somebody's phone at seven in the morning.
//!
//! They use `join_network` — an isolated network id, no relay — so they run
//! offline and in CI, and what they exercise is local discovery. The relay leg
//! has its own guard in `bin/relaycheck`, because it needs the internet and
//! must not turn CI red when a relay operator reboots.

use std::path::Path;
use std::time::Duration;

use diaswarm_core::vault::{hex, Identity, Store, Vault};
use diaswarm_core::{Record, EPOCH_MS};
use diaswarm_net::peer::add_follow;
use diaswarm_net::swarm::{network_id, Swarm};
use p2panda_core::SigningKey;

const OFFSET: i64 = 12 * 3_600_000;

fn tmp(tag: &str) -> std::path::PathBuf {
    let p = std::env::temp_dir().join(format!(
        "diaswarm-reach-{tag}-{}",
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
    ));
    std::fs::create_dir_all(&p).unwrap();
    p
}

fn day(epoch: i64, marker: f64) -> Vec<Record> {
    vec![Record::new(epoch * EPOCH_MS + 3_600_000, "cgm").set("mgdl", Some(marker.into()))]
}

/// A subject with a little history, granting one reader.
fn subject_in(store_root: &Path, reader: &Identity) -> (Identity, String) {
    let subject = Identity::generate();
    let store = Store::open(store_root).unwrap();
    let dir = store.path_for(&subject.enc_public());
    std::fs::create_dir_all(&dir).unwrap();
    let vault = Vault::create(&dir, &subject, OFFSET).unwrap();
    vault.record_grant(&subject, &reader.enc_public(), "follow", "grant", 0).unwrap();
    for i in 0..2 {
        vault.seal(22_000 + i, &day(22_000 + i, 100.0 + i as f64)).unwrap();
    }
    let id = hex(&subject.enc_public());
    (subject, id)
}

/// Try a refresh until a deadline; a discovery that needs a moment is not a
/// failure.
///
/// **A DEADLINE, NOT A TRY COUNT.** A dial to a node that is not there does not
/// return quickly — it waits out QUIC's own timeouts — so "twenty attempts"
/// is a wall-clock time nobody chose and a failing run took minutes to say so.
/// A test that is slow to fail is a test people stop running.
async fn reaches_within(follower: &Swarm, secs: u64) -> bool {
    let deadline = std::time::Instant::now() + Duration::from_secs(secs);
    while std::time::Instant::now() < deadline {
        let attempt = tokio::time::timeout(Duration::from_secs(3), follower.refresh_follows()).await;
        if let Ok(Ok(results)) = attempt {
            if results.iter().any(|r| r.reached()) {
                return true;
            }
        }
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
    false
}

/// A FOLLOWER WITH NO ADDRESS — ONLY A NODE ID — STILL FINDS THE SUBJECT.
///
/// **THIS IS WHAT AN INVITE ACTUALLY CARRIES.** `diaswarm:3:…` carries the
/// subject's node id and a relay URL, and no IP at all: an address written into
/// a QR code would be wrong the moment either phone joined a different wifi.
/// So resolving a node id to somewhere is not an optimisation, it is the only
/// route there is — and it was never tested, because every other test in this
/// crate pastes in `<id>@<ip>` captured a moment earlier.
#[tokio::test]
async fn a_node_id_with_no_address_is_enough_to_find_a_subject() {
    let net = network_id("reach-bare-id");
    let subject_store = tmp("bare-subject");
    let reader_store = tmp("bare-reader");
    let reader = Identity::generate();
    let (_subject, subject_hex) = subject_in(&subject_store, &reader);

    let publisher = Swarm::join_network(subject_store.clone(), SigningKey::generate(), net)
        .await
        .expect("publisher");
    let follower = Swarm::join_network(reader_store.clone(), SigningKey::generate(), net)
        .await
        .expect("follower");

    // NO `@address`. Just who, exactly as an invite gives it.
    let id = publisher.node_id().await.unwrap();
    add_follow(&reader_store, &subject_hex, &id, Some("follow")).unwrap();

    assert!(
        reaches_within(&follower, 20).await,
        "a follower given only a node id never found the subject — an invite carries nothing more"
    );
}

/// THE SUBJECT MOVES, AND THE FOLLOWER HAS TO FIND IT AGAIN.
///
/// **THE OVERNIGHT FAILURE, IN ONE TEST.** A phone sleeps and comes back on a
/// different address. Its node id — which is what the follower actually
/// stored — has not changed, because that is derived from a key on disk. If
/// anything in transport regresses to "reachable only at the address we were
/// handed", this is where it shows up.
///
/// The subject is rebuilt on a *new endpoint with the same signing key*, which
/// is exactly what a restart on a new network looks like from outside.
#[tokio::test]
async fn a_subject_that_changes_address_is_found_again() {
    let net = network_id("reach-moved");
    let subject_store = tmp("moved-subject");
    let reader_store = tmp("moved-reader");
    let reader = Identity::generate();
    let (_subject, subject_hex) = subject_in(&subject_store, &reader);

    let key = SigningKey::generate();
    let first = Swarm::join_network(subject_store.clone(), key.clone(), net).await.expect("first");
    let id = first.node_id().await.unwrap();

    let follower = Swarm::join_network(reader_store.clone(), SigningKey::generate(), net)
        .await
        .expect("follower");
    add_follow(&reader_store, &subject_hex, &id, Some("follow")).unwrap();
    assert!(reaches_within(&follower, 20).await, "never reached the subject in the first place");

    // The phone sleeps: the endpoint goes away entirely.
    drop(first);
    tokio::time::sleep(Duration::from_millis(500)).await;

    // And comes back — same key, so the same node id, on a fresh socket.
    let second = Swarm::join_network(subject_store, key, net).await.expect("second");
    assert_eq!(second.node_id().await.unwrap(), id, "a restart must not change the node id");

    assert!(
        reaches_within(&follower, 25).await,
        "the subject came back and the follower never found it again — this is the 2026-09-14 outage"
    );
}

/// A SWARM WITH NO USABLE RELAY STILL WORKS, AND SAYS SO.
///
/// A phone with an unreachable relay is the LAN-only phone this project had
/// before relays existed, and it still has to work at home. What it must not do
/// is claim to be relay-connected: `relay_state` is what both apps print every
/// pass, and an outage nobody can see is one nobody fixes.
#[tokio::test]
async fn a_swarm_without_a_relay_says_so_rather_than_pretending() {
    let net = network_id("reach-norelay");
    let store = tmp("norelay");
    let swarm = Swarm::join_network(store, SigningKey::generate(), net).await.expect("join");

    let state = swarm.relay_state().await;
    assert!(
        state == "none" || state == "disconnected",
        "a swarm with no relay configured reported {state:?} — it must never read as connected"
    );
}

/// AND A NODE ID NOBODY IS SERVING MUST NOT REPORT "REACHED".
///
/// **THE GUARD ON THE THREE TESTS ABOVE.** They assert that a follower finds a
/// subject; they are worth nothing unless `reached()` can also be false. A
/// shadow-mode verdict in this project once compared a number with itself and
/// reported agreement for weeks, so a positive assertion without its negative
/// is not treated as evidence here.
#[tokio::test]
async fn a_node_id_that_nobody_serves_is_not_reported_as_reached() {
    let net = network_id("reach-nobody");
    let reader_store = tmp("nobody-reader");
    let reader = Identity::generate();
    let (_subject, subject_hex) = subject_in(&reader_store, &reader);

    let follower = Swarm::join_network(reader_store.clone(), SigningKey::generate(), net)
        .await
        .expect("follower");

    // A well-formed node id belonging to an endpoint that was never started.
    let ghost = p2panda_core::SigningKey::generate().verifying_key().to_string();
    add_follow(&reader_store, &subject_hex, &ghost, Some("follow")).unwrap();

    // **HARD DEADLINE, because a dial to nowhere does not return quickly.**
    // Waiting out the QUIC timeouts took minutes; what is being asserted is
    // that it never says "reached", not how long it takes to give up. A
    // timeout here is the same answer as a false.
    assert!(
        !reaches_within(&follower, 12).await,
        "reported reaching a node that does not exist — every reachability test here is then meaningless"
    );
}

/// A **STALE** ADDRESS IS WORSE THAN NO ADDRESS, AND IT IS THE REAL CASE.
///
/// The test above follows by bare node id. A real follow record does not look
/// like that: it holds `<id>@<ip>:<port>` from the moment the invite was
/// scanned, and after the subject's phone reconnects that address belongs to
/// nobody — or, worse, to some other device on the same wifi. So the question
/// is not "can we work with no address" but "can we work through a wrong one",
/// and a dial that tries the recorded address first must not stop there.
///
/// This is the overnight outage in its exact shape.
#[tokio::test]
async fn a_recorded_address_that_has_gone_stale_does_not_block_discovery() {
    let net = network_id("reach-stale");
    let subject_store = tmp("stale-subject");
    let reader_store = tmp("stale-reader");
    let reader = Identity::generate();
    let (_subject, subject_hex) = subject_in(&subject_store, &reader);

    let key = SigningKey::generate();
    let publisher = Swarm::join_network(subject_store.clone(), key.clone(), net).await.expect("pub");
    let id = publisher.node_id().await.unwrap();

    // A plausible address that is not the publisher's: what a follow record
    // holds after the other phone has moved.
    let stale = format!("{id}@192.0.2.1:1");
    let follower = Swarm::join_network(reader_store.clone(), SigningKey::generate(), net)
        .await
        .expect("follower");
    add_follow(&reader_store, &subject_hex, &stale, Some("follow")).unwrap();

    assert!(
        reaches_within(&follower, 25).await,
        "a stale recorded address stopped the follower finding a subject that is right there"
    );
}

/// THE FOLLOWER RESTARTS AND PICKS UP WHERE IT WAS.
///
/// The other half of a restart. A follower is closed, reopened — a phone
/// rebooting, an app killed by the system overnight — and has to find the
/// subject again from what it persisted, with no scan and no help.
#[tokio::test]
async fn a_follower_that_restarts_finds_the_subject_again() {
    let net = network_id("reach-followerstart");
    let subject_store = tmp("fr-subject");
    let reader_store = tmp("fr-reader");
    let reader = Identity::generate();
    let (_subject, subject_hex) = subject_in(&subject_store, &reader);

    let publisher = Swarm::join_network(subject_store, SigningKey::generate(), net).await.expect("pub");
    let id = publisher.node_id().await.unwrap();

    let follower_key = SigningKey::generate();
    let first = Swarm::join_network(reader_store.clone(), follower_key.clone(), net)
        .await
        .expect("follower");
    add_follow(&reader_store, &subject_hex, &id, Some("follow")).unwrap();
    assert!(reaches_within(&first, 20).await, "never reached before the restart");

    drop(first);
    tokio::time::sleep(Duration::from_millis(500)).await;

    let second = Swarm::join_network(reader_store, follower_key, net).await.expect("restarted");
    assert!(
        reaches_within(&second, 25).await,
        "a restarted follower could not find a subject it had already been reading"
    );
}

/// ONE UNREACHABLE SUBJECT MUST NOT TAKE THE OTHERS DOWN WITH IT.
///
/// **THE FLAGSHIP IS A PARENT WITH MORE THAN ONE CHILD.** If a refresh gives up
/// — or worse, hangs — because one of them has a flat phone, then a second
/// child's readings stop for a reason that has nothing to do with them, and the
/// screen says only that the data is old.
#[tokio::test]
async fn one_unreachable_subject_does_not_stop_the_others() {
    let net = network_id("reach-two");
    let store = tmp("two-reader");
    let live_store = tmp("two-live");
    let reader = Identity::generate();

    let (_live, live_hex) = subject_in(&live_store, &reader);
    let (_gone, gone_hex) = subject_in(&store, &reader);

    let publisher = Swarm::join_network(live_store, SigningKey::generate(), net).await.expect("pub");
    let live_id = publisher.node_id().await.unwrap();
    let ghost = p2panda_core::SigningKey::generate().verifying_key().to_string();

    let follower = Swarm::join_network(store.clone(), SigningKey::generate(), net).await.expect("f");
    add_follow(&store, &gone_hex, &ghost, Some("follow")).unwrap();
    add_follow(&store, &live_hex, &live_id, Some("follow")).unwrap();

    // **WHAT THIS ACTUALLY GUARDS IS THE BOUND, NOT THE CONCURRENCY.** Checked
    // by mutation, because the first version of this comment claimed more than
    // the test could show: reverting the refresh to a sequential loop leaves it
    // passing, while removing the per-follow timeout fails it with
    // "refresh_follows never returned". So the load-bearing fix is that one
    // unreachable phone costs a bounded amount of time — the parallelism is
    // real and worth having, and this is not the test that proves it.
    //
    // The claim is "not prevented", not "not delayed": a pass runs every two
    // minutes, so a bounded wait is fine and an unbounded one was not.
    let results = tokio::time::timeout(Duration::from_secs(45), follower.refresh_follows())
        .await
        .expect("refresh_follows never returned — one dead follow is blocking the pass")
        .expect("refresh failed");

    assert!(
        results.iter().any(|r| r.reached()),
        "a subject with a dead phone stopped a second, healthy subject being read: {:?}",
        results.iter().map(|r| (&r.subject, r.reached())).collect::<Vec<_>>()
    );
    assert_eq!(results.len(), 2, "both follows should be reported, reachable or not");
}

/// COMING BACK FROM A LONG OUTAGE MUST CATCH UP, NOT SAMPLE.
///
/// **THE MORNING AFTER, WHICH IS THE ORDINARY CASE.** A follower's phone is off
/// or out of range all night while the subject keeps sealing. When it comes
/// back it has to collect everything it missed — and "everything" is the part
/// worth asserting, because a reader that fetches only the newest day draws a
/// graph with a hole in it and no error anywhere.
///
/// The outage is simulated the honest way round: the subject seals days while
/// the follower has no endpoint at all.
#[tokio::test]
async fn a_follower_that_was_away_collects_everything_it_missed() {
    let net = network_id("reach-catchup");
    let subject_store = tmp("catchup-subject");
    let reader_store = tmp("catchup-reader");
    let reader = Identity::generate();

    // A subject with two days, and a follower that reads them.
    let subject = Identity::generate();
    let store = Store::open(&subject_store).unwrap();
    let dir = store.path_for(&subject.enc_public());
    std::fs::create_dir_all(&dir).unwrap();
    let vault = Vault::create(&dir, &subject, OFFSET).unwrap();
    vault.record_grant(&subject, &reader.enc_public(), "follow", "grant", 0).unwrap();
    for i in 0..2 {
        vault.seal(23_000 + i, &day(23_000 + i, 100.0 + i as f64)).unwrap();
    }
    let subject_hex = hex(&subject.enc_public());

    let publisher = Swarm::join_network(subject_store.clone(), SigningKey::generate(), net)
        .await
        .expect("publisher");
    let id = publisher.node_id().await.unwrap();

    let follower = Swarm::join_network(reader_store.clone(), SigningKey::generate(), net)
        .await
        .expect("follower");
    add_follow(&reader_store, &subject_hex, &id, Some("follow")).unwrap();
    assert!(reaches_within(&follower, 20).await, "never read the subject before the outage");

    // The follower goes away. The subject keeps looping — five more days.
    drop(follower);
    for i in 2..7 {
        vault.seal(23_000 + i, &day(23_000 + i, 100.0 + i as f64)).unwrap();
    }

    // And comes back.
    let back = Swarm::join_network(reader_store.clone(), SigningKey::generate(), net)
        .await
        .expect("returned");
    assert!(reaches_within(&back, 25).await, "did not find the subject after coming back");

    // Everything, not just the newest. Seven days were sealed; seven must be
    // held, or the graph has a hole in it that nothing reports.
    let held = Store::open(&reader_store)
        .unwrap()
        .path_for(&subject.enc_public())
        .join("segments");
    let n = std::fs::read_dir(&held)
        .map(|d| d.filter_map(|e| e.ok()).filter(|e| e.path().extension().is_some_and(|x| x == "seal")).count())
        .unwrap_or(0);
    assert_eq!(n, 7, "came back from an outage holding {n} of 7 days — the rest is a silent gap");
}
