//! A follower that windows "the last N days" pulls only that window, through the
//! REAL `KeysReplicator` — the follower-side fix the on-device OOM proved
//! publisher-pruning cannot be (a retained-history carrier still serves the whole
//! log; this bounds what the follower REQUESTS).
//!
//! The flow is the one a real follower runs and the one the ingestprobe demo
//! established: sync the CONTROL log first (it is never windowed, so it brings the
//! epoch index in full), read the floor from that index with
//! `wire::floor_for_epochs`, set it on the replicator, and only THEN carry the
//! segment log — whose first session therefore advertises the floored `Have`.

use std::time::Duration;

use diaswarm_core::{Record, EPOCH_MS};
use diaswarm_keys::wire::{self, CONTROL_LOG_ID, LOG_ID};
use diaswarm_keys::Vault;
use diaswarm_net::pool;
use diaswarm_net::replicate::KeysReplicator;
use diaswarm_net::swarm::{network_id, Swarm};
use p2panda_core::{AnyOperation, Hash, SigningKey, VerifyingKey};
use p2panda_encryption::Rng;
use p2panda_store::logs::LogStore;
use p2panda_store::{SqliteStore, SqliteStoreBuilder};

const OFFSET: i64 = 12 * 3_600_000;

fn tmp(tag: &str) -> std::path::PathBuf {
    let p = std::env::temp_dir().join(format!(
        "diaswarm-keyswin-{tag}-{}",
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
    ));
    std::fs::create_dir_all(&p).unwrap();
    p
}

fn day(epoch: i64) -> Vec<Record> {
    (0..6i64)
        .map(|i| Record::new(epoch * EPOCH_MS + i * 300_000, "cgm").set("mgdl", Some((100.0 + i as f64).into())))
        .collect()
}

async fn held(store: &SqliteStore, author: &VerifyingKey, log_id: u32) -> u32 {
    let size = <SqliteStore as LogStore<AnyOperation, VerifyingKey, u32, u32, Hash>>::get_log_size(
        store, author, &log_id, None, None,
    )
    .await
    .unwrap();
    size.map(|(ops, _bytes)| ops).unwrap_or(0)
}

#[tokio::test(flavor = "multi_thread")]
async fn a_follower_holds_only_its_windowed_days() {
    let net = network_id("keys-window");
    let rng = Rng::default();

    // ---- the subject: six days, one anchor per epoch ----------------------
    let subject_key = SigningKey::generate();
    let author = subject_key.verifying_key();
    let subject_hex = author.to_hex();
    let mut subject = Vault::open(tmp("subject"), OFFSET, &subject_key).unwrap();
    let (mgr, _bundle) = Vault::key_bundle(&rng).unwrap();
    let subject_store = SqliteStoreBuilder::memory().build().await.unwrap();
    let create = subject.create(mgr).unwrap();
    wire::publish_control(&subject_store, &subject_key, &create).await.unwrap();

    const DAYS: i64 = 6;
    const KEEP_DAYS: u32 = 2;
    for e in 0..DAYS {
        let epoch = 22_000 + e;
        let seg = subject.seal(epoch, &day(epoch)).unwrap();
        let op = wire::publish(&subject_store, &subject_key, &seg).await.unwrap();
        wire::publish_anchor(&subject_store, &subject_key, &wire::Anchor { epoch, seq: op.header.seq_num })
            .await
            .unwrap();
    }
    // Segment seqs 0..5, one per epoch; the window keeps the last 2 (seqs 4,5).

    let publisher = Swarm::join_network(tmp("pub-pool"), SigningKey::generate(), net).await.unwrap();
    let (pe, pg) = publisher.parts();
    let publishing = KeysReplicator::keys(subject_store.clone(), pe, pg).await.unwrap();
    // Serve both topics, the share.rs split: segments on subject_topic, control alone.
    publishing.carry(pool::subject_topic(&subject_hex), &subject_hex).await.unwrap();
    publishing
        .carry_logs(pool::control_topic(&subject_hex), &subject_hex, &[CONTROL_LOG_ID])
        .await
        .unwrap();

    // ---- the follower: control first, then a windowed segment carry -------
    let follower_store = SqliteStoreBuilder::memory().build().await.unwrap();
    let follower = Swarm::join_network(tmp("fol-pool"), SigningKey::generate(), net).await.unwrap();
    let (fe, fg) = follower.parts();
    let following = KeysReplicator::keys(follower_store.clone(), fe, fg).await.unwrap();

    // 1) CONTROL first — brings the index (and keys), never windowed.
    following
        .carry_logs(pool::control_topic(&subject_hex), &subject_hex, &[CONTROL_LOG_ID])
        .await
        .unwrap();

    // 2) Wait for the index, then read the floor from it.
    let mut anchors = Vec::new();
    for _ in 0..60 {
        tokio::time::sleep(Duration::from_secs(1)).await;
        anchors = wire::anchors_from(&follower_store, &author).await.unwrap();
        if anchors.len() as i64 == DAYS {
            break;
        }
    }
    assert_eq!(anchors.len() as i64, DAYS, "the follower synced {} of {DAYS} anchors", anchors.len());
    let floor = wire::floor_for_epochs(&follower_store, &author, KEEP_DAYS)
        .await
        .unwrap()
        .expect("a window smaller than the history has a floor");
    assert_eq!(floor, 3, "last {KEEP_DAYS} days start after seq 3 (epochs at seqs 4,5)");

    // 3) Set the window BEFORE carrying segments, so the first segment session
    //    advertises the floored Have.
    following.window(&author, floor);

    // 4) Carry the segment log, windowed.
    following
        .carry_logs(pool::subject_topic(&subject_hex), &subject_hex, &[LOG_ID])
        .await
        .unwrap();

    // 5) The follower holds ONLY the window — not the whole six-day history.
    let mut segments = 0u32;
    for _ in 0..60 {
        tokio::time::sleep(Duration::from_secs(1)).await;
        segments = held(&follower_store, &author, LOG_ID).await;
        if segments as u32 >= KEEP_DAYS {
            break;
        }
    }
    println!("  follower events: {:?}", following.events());
    assert_eq!(
        segments, KEEP_DAYS,
        "the follower pulled {segments} segments; a {KEEP_DAYS}-day window should hold exactly {KEEP_DAYS} \
         (the publisher served all {DAYS})"
    );
    // The control log is never windowed: the create message is all there.
    assert!(held(&follower_store, &author, CONTROL_LOG_ID).await >= 1, "the control log synced in full");
}
