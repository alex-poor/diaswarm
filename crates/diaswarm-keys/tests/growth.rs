//! Does the log grow with the data, or with the number of times we publish?
//!
//! **THE MEASUREMENT THAT WOULD HAVE CAUGHT IT.** `seal` merges a batch into
//! the day and returns the merged result, and for about an hour that merged
//! result was what got published. Every flush put the whole day so far into the
//! log, so a 160 kB day cost 23 MB at a five-minute cadence — and would have
//! cost 111 MB had the cadence been shortened to fix the staleness it caused.
//!
//! Nothing caught it. The vault's own tests assert that records survive, and
//! they did: shadow mode reported `missing 0` throughout while the store grew
//! without bound. Correctness and cost are different questions and only one of
//! them was being asked.
//!
//! So this asks the other one, and asserts a ratio rather than a number,
//! because the absolute size depends on the record shape and the ratio is the
//! thing that was 144× wrong.

use diaswarm_core::{EPOCH_MS, Record};
use diaswarm_keys::{SqliteStoreBuilder, Vault, wire};
use p2panda_core::{Hash, SigningKey, VerifyingKey};
use p2panda_store::SqliteStore;
use p2panda_store::logs::LogStore;

const OFFSET: i64 = 12 * 3_600_000;

/// Bytes the subject's segment log holds for this author.
async fn log_bytes(store: &SqliteStore, author: &VerifyingKey) -> u64 {
    let size = <SqliteStore as LogStore<
        wire::KeysOperation,
        VerifyingKey,
        u32,
        u32,
        Hash,
    >>::get_log_size(store, author, &wire::LOG_ID, None, None)
    .await
    .unwrap();
    // (OPERATIONS, BYTES) — in that order, whatever the trait's doc says.
    size.map(|(_ops, bytes)| bytes as u64).unwrap_or(0)
}

fn flush(epoch: i64, from: usize, n: usize) -> Vec<Record> {
    (0..n)
        .map(|i| {
            let k = (from + i) as i64;
            Record::new(epoch * EPOCH_MS + k * 60_000, "cgm")
                .set("mgdl", Some((100.0 + (k % 80) as f64).into()))
        })
        .collect()
}

/// A DAY COSTS A DAY, HOWEVER MANY TIMES IT IS PUBLISHED.
///
/// Sixty flushes of a day — a fifth of a real cadence, enough to make the
/// difference between 1× and 60× unmistakable without a slow test.
#[tokio::test(flavor = "multi_thread")]
async fn publishing_deltas_costs_the_data_not_the_cadence() {
    let rng = diaswarm_keys::Rng::default();
    let key = SigningKey::generate();
    let root = std::env::temp_dir().join(format!(
        "diaswarm-growth-{}",
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
    ));
    std::fs::create_dir_all(&root).unwrap();

    let mut vault = Vault::open(&root, OFFSET, &key).unwrap();
    let (manager, _b) = Vault::key_bundle(&rng).unwrap();
    vault.create(manager).unwrap();
    let store = SqliteStoreBuilder::memory().build().await.unwrap();

    const FLUSHES: usize = 60;
    const PER_FLUSH: usize = 20;
    let mut plaintext = 0u64;

    for f in 0..FLUSHES {
        let batch = flush(40_000, f * PER_FLUSH, PER_FLUSH);
        plaintext += batch.iter().map(|r| r.to_canonical_json().len() as u64 + 1).sum::<u64>();

        // The subject's own copy is the merged day — that is what `seal` is for.
        vault.seal(40_000, &batch).unwrap();
        // What goes on the wire is only what is new.
        let delta = vault.seal_delta(40_000, &batch).unwrap();
        wire::publish(&store, &key, &delta).await.unwrap();
    }

    let logged = log_bytes(&store, &key.verifying_key()).await;
    let ratio = logged as f64 / plaintext as f64;
    eprintln!(
        "  {FLUSHES} flushes · {plaintext} B of records · {logged} B of log · {ratio:.2}×"
    );

    // **THE NUMBER THAT WAS 144.** AEAD and the operation header add a constant
    // per segment, so a delta log is a little over 1× and nowhere near the
    // flush count. Two is generous room for that overhead and still an order of
    // magnitude below the bug.
    assert!(
        ratio < 2.0,
        "the log grew {ratio:.1}× the data — publishing the merged day again?"
    );

    // And the other direction, so the test cannot pass by publishing nothing.
    assert!(ratio > 0.5, "the log holds {ratio:.2}× the data — is anything being published?");
}

/// AND THE MERGED DAY IS WHAT IT WOULD HAVE COST.
///
/// The same run, publishing what `seal` returns instead of the delta. Kept as a
/// measurement rather than deleted: the ratio above means nothing without the
/// number it is being compared against, and "144×" in a commit message is
/// easier to doubt than a test that produces it.
#[tokio::test(flavor = "multi_thread")]
async fn publishing_the_merged_day_costs_the_cadence() {
    let rng = diaswarm_keys::Rng::default();
    let key = SigningKey::generate();
    let root = std::env::temp_dir().join(format!(
        "diaswarm-growth-bad-{}",
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
    ));
    std::fs::create_dir_all(&root).unwrap();

    let mut vault = Vault::open(&root, OFFSET, &key).unwrap();
    let (manager, _b) = Vault::key_bundle(&rng).unwrap();
    vault.create(manager).unwrap();
    let store = SqliteStoreBuilder::memory().build().await.unwrap();

    const FLUSHES: usize = 60;
    const PER_FLUSH: usize = 20;
    let mut plaintext = 0u64;

    for f in 0..FLUSHES {
        let batch = flush(41_000, f * PER_FLUSH, PER_FLUSH);
        plaintext += batch.iter().map(|r| r.to_canonical_json().len() as u64 + 1).sum::<u64>();
        let merged = vault.seal(41_000, &batch).unwrap();
        wire::publish(&store, &key, &merged).await.unwrap();
    }

    let logged = log_bytes(&store, &key.verifying_key()).await;
    let ratio = logged as f64 / plaintext as f64;
    eprintln!("  publishing the merged day: {ratio:.1}× — this is the bug, measured");

    // Half the flush count, because the day grows linearly: the average publish
    // carries half a day. At 288 flushes that is the 144× that was shipping.
    assert!(
        ratio > (FLUSHES as f64) / 4.0,
        "expected the merged-day cost to be about half the flush count, got {ratio:.1}×"
    );
}
