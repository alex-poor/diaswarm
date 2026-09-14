//! Does a newly published operation get *pushed*, or only filed?
//!
//! **THE DEFECT THIS FILE EXISTS FOR WAS IN EVERY BUILD THIS PROJECT EVER
//! SHIPPED.** p2panda's `LogSync` catches a peer up and then, in its own words,
//! "nodes switch to live-mode to directly push new messages to the network
//! using a gossip protocol". The pushing is `SyncHandle::publish`. Nothing in
//! diaswarm ever called it: `stream` moved the handle into a spawned task as
//! `let _keep = handle` and only ever subscribed, so every operation went into
//! the store — which is not on the network — and waited for whenever the next
//! catch-up sync happened to run.
//!
//! It was invisible because the totals looked healthy. Operations kept
//! arriving, `carrying 2 log(s)` kept being reported, and a follower was
//! minutes stale for reasons that got blamed on doze, on Wi-Fi locks, on the
//! relay and on a one-shot subscription in turn. What said it out loud was
//! `received_live_operations: 0` in all sixty-nine live-mode sessions on two
//! phones.
//!
//! So these assert on the two halves separately, which is the only way the
//! difference is visible: `received` counts everything that arrived, `live`
//! counts what arrived pushed. A build with the send half missing passes the
//! first and fails the second.

use std::time::Duration;

use diaswarm_core::{EPOCH_MS, Record};
use diaswarm_keys::wire::{self, KeysOperation, LOG_ID};
use diaswarm_keys::Vault;
use diaswarm_net::pool;
use diaswarm_net::replicate::KeysReplicator;
use diaswarm_net::swarm::{network_id, Swarm};
use p2panda_core::{Hash, SigningKey, VerifyingKey};
use p2panda_encryption::Rng;
use p2panda_store::logs::LogStore;
use p2panda_store::{SqliteStore, SqliteStoreBuilder};

const OFFSET: i64 = 12 * 3_600_000;

fn tmp(tag: &str) -> std::path::PathBuf {
    let p = std::env::temp_dir().join(format!(
        "diaswarm-live-{tag}-{}",
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
    ));
    std::fs::create_dir_all(&p).unwrap();
    p
}

fn day(epoch: i64, mgdl: f64) -> Vec<Record> {
    (0..6i64)
        .map(|i| {
            Record::new(epoch * EPOCH_MS + i * 300_000, "cgm")
                .set("mgdl", Some((mgdl + i as f64).into()))
        })
        .collect()
}

async fn segments_held(store: &SqliteStore, author: &VerifyingKey) -> u32 {
    let size = <SqliteStore as LogStore<KeysOperation, VerifyingKey, u32, u32, Hash>>::get_log_size(
        store, author, &LOG_ID, None, None,
    )
    .await
    .unwrap();
    size.map(|(ops, _bytes)| ops).unwrap_or(0)
}

/// A SEGMENT SEALED AFTER THE PEER IS ALREADY LISTENING ARRIVES PUSHED.
///
/// The shape of the real thing: a phone that has been following all day, and a
/// subject that seals one more five-minute batch. Before this test existed,
/// that batch reached the follower when the next catch-up sync ran — which is
/// why freshness on hardware tracked the sync interval and never the publish.
#[tokio::test(flavor = "multi_thread")]
async fn a_segment_published_now_is_pushed_now() {
    let net = network_id("keys-live-push");
    let rng = Rng::default();

    let subject_key = SigningKey::generate();
    let author = subject_key.verifying_key();
    let subject_hex = author.to_hex();
    let mut subject = Vault::open(tmp("subject"), OFFSET, &subject_key).unwrap();
    let (mgr, _bundle) = Vault::key_bundle(&rng).unwrap();

    let subject_store = SqliteStoreBuilder::memory().build().await.unwrap();
    let create = subject.create(mgr).unwrap();
    wire::publish_control(&subject_store, &subject_key, &create).await.unwrap();

    // A little history, so there is something for catch-up to do. If catch-up
    // never ran, "nothing arrived live" would be ambiguous.
    let history = 2i64;
    for e in 0..history {
        let segment = subject.seal(23_000 + e, &day(23_000 + e, 100.0)).unwrap();
        wire::publish(&subject_store, &subject_key, &segment).await.unwrap();
    }

    let publisher = Swarm::join_network(tmp("pub"), SigningKey::generate(), net).await.unwrap();
    let (pe, pg) = publisher.parts();
    let publishing = KeysReplicator::keys(subject_store.clone(), pe, pg).await.unwrap();

    let carrier_store = SqliteStoreBuilder::memory().build().await.unwrap();
    let carrier = Swarm::join_network(tmp("car"), SigningKey::generate(), net).await.unwrap();
    let (ce, cg) = carrier.parts();
    let carrying = KeysReplicator::keys(carrier_store.clone(), ce, cg).await.unwrap();

    let depth = pool::depth_for(2);
    let topic = pool::bucket_topic(depth, pool::bucket_of(&subject_hex, depth));
    publishing.carry(topic, &subject_hex).await.unwrap();
    carrying.carry(topic, &subject_hex).await.unwrap();

    // ---- catch-up runs first, and the live counter must still be zero ------
    let mut caught_up = 0;
    for _ in 0..60 {
        tokio::time::sleep(Duration::from_secs(1)).await;
        caught_up = segments_held(&carrier_store, &author).await;
        if caught_up as i64 >= history {
            break;
        }
    }
    assert_eq!(
        caught_up as i64, history,
        "catch-up never delivered the existing {history} segments, so nothing below \
         this line can be interpreted (events: {:?})",
        carrying.events()
    );
    assert_eq!(
        carrying.live_received(),
        0,
        "history arrived in the live phase, which means this test is not \
         measuring what it claims to"
    );

    // ---- and now the thing a follower is waiting for ----------------------
    let fresh = subject.seal(23_000 + history, &day(23_000 + history, 140.0)).unwrap();
    let operation = wire::publish(&subject_store, &subject_key, &fresh).await.unwrap();
    let sent = publishing.broadcast(&subject_hex, operation);
    assert_eq!(
        sent, 1,
        "the publisher had no live stream to push onto — `carry` streamed the \
         topic, so a 0 here means the handle was dropped or never kept"
    );

    let mut live = 0;
    let mut held = caught_up;
    for _ in 0..45 {
        tokio::time::sleep(Duration::from_secs(1)).await;
        live = carrying.live_received();
        held = segments_held(&carrier_store, &author).await;
        if live >= 1 && held as i64 > history {
            break;
        }
    }
    println!("  publisher events: {:?}", publishing.events());
    println!("  carrier events:   {:?}", carrying.events());
    assert!(
        live >= 1,
        "a segment published while the peer was already in live mode arrived \
         {live} times over gossip in 45s. `received_live_operations` was 0 in \
         every one of sixty-nine live-mode sessions on two phones for exactly \
         this reason: nothing ever called `SyncHandle::publish`"
    );
    assert!(
        held as i64 > history,
        "the pushed segment was received but not stored: {held} segments held, \
         {history} before the push"
    );
    // **NO `live <= received` ASSERTION HERE, AND THE REASON MATTERS.** They
    // measure different things now: `live` is p2panda's own count of live
    // arrivals per session, summed, and `received` is how many operations this
    // peer stored. A duplicate delivered live on two sessions is two arrivals
    // and one stored operation. The check that does hold is against the
    // publisher's own count, and it lives in `tests/two_process.rs` where both
    // sides can be seen at once.
}

/// AND A GRANT DOES TOO, WHICH IS THE ONE THAT STRANDS A NEW READER.
///
/// A welcome sits in the control log, and a reader that has not received it
/// cannot open a single day however many segments it holds. That is the failure
/// that reads as a broken reader and is a missing writer. Pairing is the moment
/// somebody is watching, so it is the worst moment to wait for the next
/// catch-up sync.
#[tokio::test(flavor = "multi_thread")]
async fn a_grant_published_now_is_pushed_now() {
    let net = network_id("keys-live-grant");
    let rng = Rng::default();

    let subject_key = SigningKey::generate();
    let author = subject_key.verifying_key();
    let subject_hex = author.to_hex();
    let mut subject = Vault::open(tmp("g-subject"), OFFSET, &subject_key).unwrap();
    let (mgr, _bundle) = Vault::key_bundle(&rng).unwrap();
    let (_reader_mgr, reader_bundle) = Vault::key_bundle(&rng).unwrap();

    let subject_store = SqliteStoreBuilder::memory().build().await.unwrap();
    let create = subject.create(mgr).unwrap();
    wire::publish_control(&subject_store, &subject_key, &create).await.unwrap();

    let publisher = Swarm::join_network(tmp("g-pub"), SigningKey::generate(), net).await.unwrap();
    let (pe, pg) = publisher.parts();
    let publishing = KeysReplicator::keys(subject_store.clone(), pe, pg).await.unwrap();

    let carrier_store = SqliteStoreBuilder::memory().build().await.unwrap();
    let carrier = Swarm::join_network(tmp("g-car"), SigningKey::generate(), net).await.unwrap();
    let (ce, cg) = carrier.parts();
    let carrying = KeysReplicator::keys(carrier_store.clone(), ce, cg).await.unwrap();

    let depth = pool::depth_for(2);
    let topic = pool::bucket_topic(depth, pool::bucket_of(&subject_hex, depth));
    publishing.carry(topic, &subject_hex).await.unwrap();
    carrying.carry(topic, &subject_hex).await.unwrap();

    // Catch up on the group's creation, so the session is past its sync phase.
    let mut controls = 0;
    for _ in 0..60 {
        tokio::time::sleep(Duration::from_secs(1)).await;
        controls = wire::control_from(&carrier_store, &author, None).await.unwrap().len();
        if controls >= 1 {
            break;
        }
    }
    assert_eq!(controls, 1, "catch-up never delivered the group's creation");

    let (welcome, _tag) = subject.grant(reader_bundle, "follow").unwrap();
    let operation = wire::publish_control(&subject_store, &subject_key, &welcome).await.unwrap();
    assert_eq!(publishing.broadcast(&subject_hex, operation), 1, "no live stream to push a grant onto");

    let mut live = 0;
    for _ in 0..45 {
        tokio::time::sleep(Duration::from_secs(1)).await;
        live = carrying.live_received();
        controls = wire::control_from(&carrier_store, &author, None).await.unwrap().len();
        if live >= 1 && controls >= 2 {
            break;
        }
    }
    println!("  carrier events: {:?}", carrying.events());
    assert!(live >= 1, "the welcome was not pushed: it reaches a new reader at the next catch-up");
    assert_eq!(controls, 2, "the pushed welcome did not land in the control log");
}

/// BROADCASTING A SUBJECT NOBODY CARRIES IS 0, NOT AN ERROR.
///
/// **A GUARD AGAINST MAKING THE COUNT MEANINGLESS.** The easiest way to make
/// `pushed=` always look healthy is to have `broadcast` push onto every topic
/// it happens to hold a handle for. Then a publisher following somebody else
/// would report pushes for a subject nobody asked it about, and the number that
/// exists to detect a broken send half would never be zero again.
#[tokio::test(flavor = "multi_thread")]
async fn pushing_a_subject_nobody_carries_says_nobody() {
    let net = network_id("keys-live-nobody");
    let subject_key = SigningKey::generate();
    let subject_hex = subject_key.verifying_key().to_hex();
    let store = SqliteStoreBuilder::memory().build().await.unwrap();

    let mut vault = Vault::open(tmp("n-subject"), OFFSET, &subject_key).unwrap();
    let (mgr, _b) = Vault::key_bundle(&Rng::default()).unwrap();
    let create = vault.create(mgr).unwrap();
    let operation = wire::publish_control(&store, &subject_key, &create).await.unwrap();

    let swarm = Swarm::join_network(tmp("n-pub"), SigningKey::generate(), net).await.unwrap();
    let (e, g) = swarm.parts();
    let replicator = KeysReplicator::keys(store.clone(), e, g).await.unwrap();

    // Streaming a bucket is not carrying this subject: `carry` records the pair.
    let other = SigningKey::generate().verifying_key().to_hex();
    let depth = pool::depth_for(2);
    replicator
        .carry(pool::bucket_topic(depth, pool::bucket_of(&other, depth)), &other)
        .await
        .unwrap();

    assert_eq!(
        replicator.broadcast(&subject_hex, operation),
        0,
        "an operation went out on a topic that was streamed for a different subject"
    );
}

/// AND PUSHING STILL WORKS AFTER A RE-SUBSCRIBE.
///
/// **THE INTERACTION BETWEEN THE TWO REPAIRS, WHICH IS WHERE THE NEXT SILENT
/// FAILURE WOULD LIVE.** `restream` exists because a one-shot subscription goes
/// quiet when a gossip link dies and nothing re-establishes it; it works by
/// dropping each topic's handle, which is what unsubscribes. `broadcast` works
/// by looking that same handle up. So the two now share a map, and the
/// failure mode if they ever get out of step is the worst kind: `pushed=1`
/// against a handle whose actor is gone, reported as success, delivering
/// nothing. Exactly the shape of the defect this file was written for.
///
/// A phone re-streams after a stall and then keeps publishing every five
/// minutes, so this is the ordinary path and not an edge case.
#[tokio::test(flavor = "multi_thread")]
async fn a_re_subscribe_does_not_quietly_end_pushing() {
    let net = network_id("keys-live-restream");
    let rng = Rng::default();

    let subject_key = SigningKey::generate();
    let author = subject_key.verifying_key();
    let subject_hex = author.to_hex();
    let mut subject = Vault::open(tmp("r-subject"), OFFSET, &subject_key).unwrap();
    let (mgr, _bundle) = Vault::key_bundle(&rng).unwrap();

    let subject_store = SqliteStoreBuilder::memory().build().await.unwrap();
    let create = subject.create(mgr).unwrap();
    wire::publish_control(&subject_store, &subject_key, &create).await.unwrap();
    let segment = subject.seal(26_000, &day(26_000, 100.0)).unwrap();
    wire::publish(&subject_store, &subject_key, &segment).await.unwrap();

    let publisher = Swarm::join_network(tmp("r-pub"), SigningKey::generate(), net).await.unwrap();
    let (pe, pg) = publisher.parts();
    let publishing = KeysReplicator::keys(subject_store.clone(), pe, pg).await.unwrap();

    let carrier_store = SqliteStoreBuilder::memory().build().await.unwrap();
    let carrier = Swarm::join_network(tmp("r-car"), SigningKey::generate(), net).await.unwrap();
    let (ce, cg) = carrier.parts();
    let carrying = KeysReplicator::keys(carrier_store.clone(), ce, cg).await.unwrap();

    let depth = pool::depth_for(2);
    let topic = pool::bucket_topic(depth, pool::bucket_of(&subject_hex, depth));
    publishing.carry(topic, &subject_hex).await.unwrap();
    carrying.carry(topic, &subject_hex).await.unwrap();

    let mut caught_up = 0;
    for _ in 0..60 {
        tokio::time::sleep(Duration::from_secs(1)).await;
        caught_up = segments_held(&carrier_store, &author).await;
        if caught_up >= 1 {
            break;
        }
    }
    assert_eq!(caught_up, 1, "catch-up never ran, so nothing below can be read");

    // THE STALL REMEDY, on both sides — a phone re-streams its own topics, and
    // the publisher is a peer like any other.
    assert_eq!(publishing.restream().await.unwrap(), 1, "the publisher re-streamed nothing");
    assert_eq!(carrying.restream().await.unwrap(), 1, "the carrier re-streamed nothing");

    // Give the fresh subscriptions time to find each other again.
    tokio::time::sleep(Duration::from_secs(10)).await;

    let before = carrying.live_received();
    let fresh = subject.seal(26_001, &day(26_001, 150.0)).unwrap();
    let operation = wire::publish(&subject_store, &subject_key, &fresh).await.unwrap();
    assert_eq!(
        publishing.broadcast(&subject_hex, operation),
        1,
        "after re-streaming, the publisher had no handle to push onto — the \
         handle `restream` dropped was never replaced in the map `broadcast` reads"
    );

    let mut live = before;
    let mut held = caught_up;
    for _ in 0..45 {
        tokio::time::sleep(Duration::from_secs(1)).await;
        live = carrying.live_received();
        held = segments_held(&carrier_store, &author).await;
        if live > before && held > caught_up {
            break;
        }
    }
    println!("  carrier events: {:?}", carrying.events());
    assert!(
        live > before,
        "nothing was pushed after a re-subscribe: `pushed` would have said 1 \
         while delivering nothing, which is the failure this whole file exists \
         to make impossible"
    );
    assert!(held > caught_up, "the pushed segment was received but not stored");
}
