//! Pruning the segment log keeps the recent window and drops the rest (D35).
//!
//! A fresh follower syncs every operation a subject holds, which OOM-crashed the
//! app against a real-history subject (30 MB → 3.4 GB, native SIGABRT). p2panda is
//! built assuming apps prune; we never did. [`wire::prune_segments`] is the fix.
//! `ingestprobe` measures that pruning bounds the *sync memory*; this asserts the
//! on-disk mechanism it rests on — the right ops are dropped, the control log is
//! left alone, and what survives is still readable.

use diaswarm_core::{EPOCH_MS, Record};
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
async fn prune_keeps_the_recent_window_and_leaves_the_control_log() {
    let rng = diaswarm_keys::Rng::default();
    let key = SigningKey::generate();
    let author = key.verifying_key();
    let root = std::env::temp_dir().join(format!("diaswarm-prune-{}", nanos()));
    std::fs::create_dir_all(&root).unwrap();

    let mut vault = Vault::open(&root, OFFSET, &key).unwrap();
    let (manager, _b) = Vault::key_bundle(&rng).unwrap();
    let create = vault.create(manager).unwrap();
    let store = SqliteStoreBuilder::memory().build().await.unwrap();
    wire::publish_control(&store, &key, &create).await.unwrap();

    // 100 one-reading delta segments → 100 operations in the segment log.
    const N: u64 = 100;
    const KEEP: u64 = 20;
    for i in 0..N as i64 {
        let batch = vec![
            Record::new(40_000 * EPOCH_MS + i * 60_000, "cgm").set("mgdl", Some((100.0 + (i % 50) as f64).into())),
        ];
        let delta = vault.seal_delta(40_000, &batch).unwrap();
        wire::publish(&store, &key, &delta).await.unwrap();
    }
    assert_eq!(ops(&store, &author, wire::LOG_ID).await, N, "sealed N segments");
    let control_before = ops(&store, &author, wire::CONTROL_LOG_ID).await;
    assert!(control_before > 0, "the create control op should be in the control log");

    // Prune to the last KEEP.
    let pruned = wire::prune_segments(&store, &author, KEEP).await.unwrap();
    assert_eq!(pruned, N - KEEP, "pruned everything but the kept window");
    assert_eq!(ops(&store, &author, wire::LOG_ID).await, KEEP, "segment log holds only the window");

    // The control log — grants and epoch keys a follower needs to open anything —
    // is untouched.
    assert_eq!(
        ops(&store, &author, wire::CONTROL_LOG_ID).await,
        control_before,
        "control log must not be pruned"
    );

    // What survived is the most RECENT window and is still readable.
    let tail = wire::segments_tail(&store, &author, KEEP).await.unwrap();
    assert_eq!(tail.len() as u64, KEEP, "the kept window reads back in full");

    // Pruning again at or below the window is a no-op.
    assert_eq!(wire::prune_segments(&store, &author, KEEP).await.unwrap(), 0, "no-op below the window");
    assert_eq!(wire::prune_segments(&store, &author, KEEP + 1000).await.unwrap(), 0, "keep > held is a no-op");
}
