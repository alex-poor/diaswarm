//! The epoch→segment-seq index a windowing follower reads (step 2).
//!
//! A follower that wants "the last N days" needs the segment sequence each epoch
//! begins at — but that lives in the segment headers it is trying NOT to mirror.
//! So the publisher copies each boundary into the CONTROL log (always fully synced)
//! as an [`Anchor`]. This asserts the three things that rests on: anchors ride the
//! control log without breaking the message reader, they read back authenticated,
//! and the window floor they yield is by epoch VALUE (a quiet day still counts).

use diaswarm_core::{Record, EPOCH_MS};
use diaswarm_keys::{wire, SqliteStoreBuilder, Vault};
use p2panda_core::{Hash, SigningKey, VerifyingKey};
use p2panda_store::logs::LogStore;
use p2panda_store::SqliteStore;

const OFFSET: i64 = 12 * 3_600_000;

async fn ops(store: &SqliteStore, author: &VerifyingKey, log_id: u32) -> u64 {
    let size = <SqliteStore as LogStore<
        p2panda_core::AnyOperation,
        VerifyingKey,
        u32,
        u32,
        Hash,
    >>::get_log_size(store, author, &log_id, None, None)
    .await
    .unwrap();
    size.map(|(o, _bytes)| o as u64).unwrap_or(0)
}

fn nanos() -> u128 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
}

#[tokio::test(flavor = "multi_thread")]
async fn anchors_index_epochs_and_the_message_reader_ignores_them() {
    let rng = diaswarm_keys::Rng::default();
    let key = SigningKey::generate();
    let author = key.verifying_key();
    let root = std::env::temp_dir().join(format!("diaswarm-epoch-index-{}", nanos()));
    std::fs::create_dir_all(&root).unwrap();

    let mut vault = Vault::open(&root, OFFSET, &key).unwrap();
    let (manager, _b) = Vault::key_bundle(&rng).unwrap();
    let create = vault.create(manager).unwrap();
    let store = SqliteStoreBuilder::memory().build().await.unwrap();
    wire::publish_control(&store, &key, &create).await.unwrap();

    // Three epochs, two segments each. Capture the seq each epoch starts at and
    // publish an anchor for it — exactly what the publisher will do at rotation.
    let epochs = [100_i64, 101, 102];
    let mut starts = Vec::new();
    for &epoch in &epochs {
        let batch = vec![Record::new(epoch * EPOCH_MS, "cgm").set("mgdl", Some(100.0.into()))];
        let first = vault.seal_delta(epoch, &batch).unwrap();
        let seq = wire::publish(&store, &key, &first).await.unwrap().header.seq_num;
        starts.push(seq);
        wire::publish_anchor(&store, &key, &wire::Anchor { epoch, seq }).await.unwrap();
        // a second segment in the same epoch, no anchor
        let batch = vec![Record::new(epoch * EPOCH_MS + 60_000, "cgm").set("mgdl", Some(101.0.into()))];
        let more = vault.seal_delta(epoch, &batch).unwrap();
        wire::publish(&store, &key, &more).await.unwrap();
    }
    // seq layout: epoch100@0, epoch101@2, epoch102@4.
    assert_eq!(starts, vec![0, 2, 4], "each epoch starts where expected");
    assert_eq!(ops(&store, &author, wire::LOG_ID).await, 6, "six segments");
    assert_eq!(
        ops(&store, &author, wire::CONTROL_LOG_ID).await,
        4,
        "control log holds the create message + three anchors"
    );

    // The message reader skips anchors: it still returns only the create message,
    // and does NOT error trying to decode an anchor body as a message.
    let messages = wire::control_from(&store, &author, None).await.unwrap();
    assert_eq!(messages.len(), 1, "control_from returns only the create message, anchors skipped");

    // The index reads back, authenticated and epoch-sorted.
    let anchors = wire::anchors_from(&store, &author).await.unwrap();
    assert_eq!(
        anchors,
        vec![
            wire::Anchor { epoch: 100, seq: 0 },
            wire::Anchor { epoch: 101, seq: 2 },
            wire::Anchor { epoch: 102, seq: 4 },
        ],
        "every anchor, sorted by epoch"
    );

    // The window floor is Some(start - 1) so a Have at it pulls that segment and on.
    assert_eq!(wire::floor_for_epochs(&store, &author, 1).await.unwrap(), Some(3), "last 1 epoch → from seq 4");
    assert_eq!(wire::floor_for_epochs(&store, &author, 2).await.unwrap(), Some(1), "last 2 epochs → from seq 2");
    assert_eq!(wire::floor_for_epochs(&store, &author, 3).await.unwrap(), None, "window spans the whole index");
    assert_eq!(wire::floor_for_epochs(&store, &author, 10).await.unwrap(), None, "keep > epochs → no trim");
    assert_eq!(wire::floor_for_epochs(&store, &author, 0).await.unwrap(), None, "keep 0 → unset, no trim");
}

#[tokio::test(flavor = "multi_thread")]
async fn window_is_by_epoch_value_so_a_quiet_day_still_counts() {
    let rng = diaswarm_keys::Rng::default();
    let key = SigningKey::generate();
    let author = key.verifying_key();
    let root = std::env::temp_dir().join(format!("diaswarm-epoch-gap-{}", nanos()));
    std::fs::create_dir_all(&root).unwrap();

    let mut vault = Vault::open(&root, OFFSET, &key).unwrap();
    let (manager, _b) = Vault::key_bundle(&rng).unwrap();
    let create = vault.create(manager).unwrap();
    let store = SqliteStoreBuilder::memory().build().await.unwrap();
    wire::publish_control(&store, &key, &create).await.unwrap();

    // Epochs 200, 202, 203 — epoch 201 produced NO readings (a quiet day).
    for (i, &epoch) in [200_i64, 202, 203].iter().enumerate() {
        let batch = vec![Record::new(epoch * EPOCH_MS, "cgm").set("mgdl", Some(100.0.into()))];
        let first = vault.seal_delta(epoch, &batch).unwrap();
        let seq = wire::publish(&store, &key, &first).await.unwrap().header.seq_num;
        assert_eq!(seq, i as u32, "one segment per epoch here");
        wire::publish_anchor(&store, &key, &wire::Anchor { epoch, seq }).await.unwrap();
    }
    // anchors: {200,0},{202,1},{203,2}. Latest=203.
    // keep=2 → cutoff = 203-1 = 202. First anchor >= 202 is {202,1} → Some(0).
    // The quiet day 201 is still spent: we do NOT fall back to {200,0}.
    assert_eq!(wire::floor_for_epochs(&store, &author, 2).await.unwrap(), Some(0), "cutoff 202 lands on the 202 anchor, not 200");
    // keep=3 → cutoff = 201; earliest anchor is 200 < 201, first >= 201 is {202,1} → Some(0).
    assert_eq!(wire::floor_for_epochs(&store, &author, 3).await.unwrap(), Some(0), "cutoff 201 still trims epoch 200");
    // keep=4 → cutoff = 200 = earliest, and earliest sits at seq 0 (the index starts
    // at the head of the log) → nothing to trim.
    assert_eq!(wire::floor_for_epochs(&store, &author, 4).await.unwrap(), None, "cutoff reaches a seq-0 start → no trim");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_recently_anchoring_publisher_windows_to_its_oldest_anchor_not_the_whole_log() {
    // The real transition: a publisher that sealed history BEFORE it learned to
    // anchor. Its oldest anchor sits mid-log (seq > 0) with un-anchored segments
    // beneath it. A windowing follower must bound to that oldest anchor — NOT fall
    // back to syncing the whole log, which is the unbounded ingest the window exists
    // to stop (the on-device OOM). Without this, a window does not engage until the
    // index has accrued more epochs than the window spans — days, for a week-long
    // window — and the follower OOMs in the meantime.
    let rng = diaswarm_keys::Rng::default();
    let key = SigningKey::generate();
    let author = key.verifying_key();
    let root = std::env::temp_dir().join(format!("diaswarm-epoch-accrual-{}", nanos()));
    std::fs::create_dir_all(&root).unwrap();

    let mut vault = Vault::open(&root, OFFSET, &key).unwrap();
    let (manager, _b) = Vault::key_bundle(&rng).unwrap();
    let create = vault.create(manager).unwrap();
    let store = SqliteStoreBuilder::memory().build().await.unwrap();
    wire::publish_control(&store, &key, &create).await.unwrap();

    // Three pre-anchor segments (epochs 400, 400, 401) — no anchors, as the old
    // build produced. They land at seq 0, 1, 2.
    for (i, &(epoch, t)) in [(400_i64, 0_i64), (400, 60_000), (401, 0)].iter().enumerate() {
        let batch = vec![Record::new(epoch * EPOCH_MS + t, "cgm").set("mgdl", Some(100.0.into()))];
        let seq = wire::publish(&store, &key, &vault.seal_delta(epoch, &batch).unwrap())
            .await
            .unwrap()
            .header
            .seq_num;
        assert_eq!(seq, i as u32, "pre-anchor segment lands in sequence");
    }

    // The publisher upgrades: epoch 402 is the first it anchors. Its first segment
    // lands at seq 3, above the pre-anchor history, and the anchor records that.
    let batch = vec![Record::new(402 * EPOCH_MS, "cgm").set("mgdl", Some(100.0.into()))];
    let seq = wire::publish(&store, &key, &vault.seal_delta(402, &batch).unwrap())
        .await
        .unwrap()
        .header
        .seq_num;
    assert_eq!(seq, 3, "first anchored segment lands above the pre-anchor history");
    wire::publish_anchor(&store, &key, &wire::Anchor { epoch: 402, seq }).await.unwrap();
    // A second epoch-402 segment, no anchor.
    let batch = vec![Record::new(402 * EPOCH_MS + 60_000, "cgm").set("mgdl", Some(101.0.into()))];
    wire::publish(&store, &key, &vault.seal_delta(402, &batch).unwrap()).await.unwrap();

    // Exactly one anchor, sitting at seq 3 (NOT seq 0).
    assert_eq!(
        wire::anchors_from(&store, &author).await.unwrap(),
        vec![wire::Anchor { epoch: 402, seq: 3 }],
        "one anchor, mid-log",
    );

    // keep=1: the whole index is a single epoch, so the cutoff reaches the oldest
    // anchor. It must STILL bound to that anchor (floor = seq 3 - 1 = 2), so the
    // follower pulls only epoch 402 (seq 3, 4) and none of the pre-anchor history at
    // seq 0..2. The pre-fix code returned None here and full-followed the lot.
    assert_eq!(
        wire::floor_for_epochs(&store, &author, 1).await.unwrap(),
        Some(2),
        "single mid-log anchor → bound to it, do not full-follow the pre-anchor log",
    );
    // Asking for MORE days than are anchored still bounds to the oldest anchor, never
    // None: it under-delivers history that has no anchor yet, but never re-opens the
    // unbounded ingest. As anchors accrue the window widens on its own.
    assert_eq!(
        wire::floor_for_epochs(&store, &author, 30).await.unwrap(),
        Some(2),
        "window wider than the index → still bounded to the oldest anchor",
    );
}
