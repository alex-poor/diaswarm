//! `follow::read_since` fetches, verifies and decrypts only what arrived after
//! a sequence-number mark — not the whole window every poll.
//!
//! **THE COST THIS GUARDS.** `follow::read` re-reads its window on every call;
//! `tailcost` measured that at 64% of a phone's native cycles and 28% of its
//! allocator, and on a healthy link every new reading invalidates the cache and
//! pays the window again — a native heap that climbs on an undisturbed phone
//! carrying a handful of readings. This asserts the incremental path does the
//! work once: after a mark, a poll opens only the new segments.

use diaswarm_core::{EPOCH_MS, Record};
use diaswarm_keys::{follow, wire, Rng, SqliteStoreBuilder, Vault};
use p2panda_core::SigningKey;

const OFFSET: i64 = 12 * 3_600_000;

fn tmp(tag: &str) -> std::path::PathBuf {
    let p = std::env::temp_dir().join(format!(
        "diaswarm-incr-{tag}-{}",
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
    ));
    std::fs::create_dir_all(&p).unwrap();
    p
}

fn readings(epoch: i64, from_slot: i64, n: i64) -> Vec<Record> {
    (0..n)
        .map(|i| {
            let slot = from_slot + i;
            Record::new(epoch * EPOCH_MS + slot * 60_000, "cgm")
                .set("mgdl", Some((100.0 + slot as f64).into()))
        })
        .collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn read_since_opens_only_what_arrived_after_the_mark() {
    let key = SigningKey::from_bytes(&[7u8; 32]);
    let author = key.verifying_key();
    let rng = Rng::default();
    let mut vault = Vault::open(tmp("subject"), OFFSET, &key).unwrap();
    let (mgr, _bundle) = Vault::key_bundle(&rng).unwrap();
    vault.create(mgr).unwrap();
    let store = SqliteStoreBuilder::memory().build().await.unwrap();

    // Seal three segments on day 40_000.
    for f in 0..3i64 {
        let seg = vault.seal_delta(40_000, &readings(40_000, f * 10, 10)).unwrap();
        wire::publish(&store, &key, &seg).await.unwrap();
    }

    // First read: no mark -> the whole log, 3 segments, 30 records.
    let (recs0, opened0, skip0, tip0) =
        follow::read_since(&vault, &store, &author, None).await.unwrap();
    assert_eq!(opened0, 3, "first read should open all three segments");
    assert_eq!(recs0.len(), 30, "first read should return all thirty readings");
    assert_eq!(skip0.lost(), 0, "nothing should fail to open");
    // Sequence numbers are 0-indexed, so three segments put the tip at 2.
    assert_eq!(tip0, 2, "tip is the newest seq of the three segments");

    // A poll with no new data opens NOTHING.
    let (recs_q, opened_q, _s, tip_q) =
        follow::read_since(&vault, &store, &author, Some(tip0)).await.unwrap();
    assert_eq!(opened_q, 0, "a poll with no new data must open no segments");
    assert!(recs_q.is_empty(), "a quiet poll returns no records");
    assert_eq!(tip_q, tip0, "tip unchanged when nothing arrived");

    // Two more segments arrive.
    for f in 3..5i64 {
        let seg = vault.seal_delta(40_000, &readings(40_000, f * 10, 10)).unwrap();
        wire::publish(&store, &key, &seg).await.unwrap();
    }

    // The incremental poll opens ONLY the two new segments, not all five.
    let (recs1, opened1, skip1, tip1) =
        follow::read_since(&vault, &store, &author, Some(tip0)).await.unwrap();
    assert_eq!(opened1, 2, "poll after mark opens only the two new segments, not all five");
    assert_eq!(recs1.len(), 20, "poll returns only the twenty new readings");
    assert_eq!(skip1.lost(), 0, "nothing should fail to open");
    assert!(tip1 > tip0, "tip advanced ({tip0} -> {tip1})");

    // The full window read still sees everything (the audited path is intact).
    let (all, opened_all, _s, _t) =
        follow::read_since(&vault, &store, &author, None).await.unwrap();
    assert_eq!(opened_all, 5, "a fresh full read still opens all five");
    assert_eq!(all.len(), 50, "a fresh full read returns all fifty");
}

/// The reader memo keys on BOTH log tips, so a grant/rotation — which touches
/// only the control log — invalidates a cached read. Without this, a reader
/// that cached a window before a rotation reached it would serve the stale
/// result for ever (the 2026-09-16 "outside your window" bug). This asserts the
/// signal the memo depends on: a grant moves the control tip and NOT the
/// segment tip.
#[tokio::test(flavor = "multi_thread")]
async fn a_grant_moves_the_control_tip_but_not_the_segment_tip() {
    let key = SigningKey::from_bytes(&[9u8; 32]);
    let author = key.verifying_key();
    let rng = Rng::default();
    let mut vault = Vault::open(tmp("grant-subject"), OFFSET, &key).unwrap();
    let (mgr, _bundle) = Vault::key_bundle(&rng).unwrap();
    let create = vault.create(mgr).unwrap();
    let store = SqliteStoreBuilder::memory().build().await.unwrap();
    wire::publish_control(&store, &key, &create).await.unwrap();

    for f in 0..2i64 {
        let seg = vault.seal_delta(40_000, &readings(40_000, f * 10, 10)).unwrap();
        wire::publish(&store, &key, &seg).await.unwrap();
    }
    let (seg0, ctrl0) = wire::log_tips(&store, &author).await.unwrap();

    // Grant a brand-new reader: a control-log operation, no new segment.
    let (_reader_mgr, reader_bundle) = Vault::key_bundle(&rng).unwrap();
    let (welcome, _tag) = vault.grant(reader_bundle, "follow").unwrap();
    wire::publish_control(&store, &key, &welcome).await.unwrap();

    let (seg1, ctrl1) = wire::log_tips(&store, &author).await.unwrap();
    assert_eq!(seg1, seg0, "a grant must not move the segment tip");
    assert!(ctrl1 > ctrl0, "a grant must move the control tip ({ctrl0} -> {ctrl1})");
}
