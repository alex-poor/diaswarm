//! Does wrapping a segment in a p2panda operation cost anything?
//!
//! [D26](../../../docs/decisions.md) argues that replication can be
//! `p2panda-net`'s log sync without reintroducing the chain, because the chain
//! was `p2panda-spaces`' CRDT and not the operation. That was reasoned from
//! where `Ingested`'s timings sat. This measures it.

use std::time::Instant;

use diaswarm_core::{EPOCH_MS, Record};
use diaswarm_keys::{Vault, wire};
use p2panda_core::SigningKey;
use p2panda_encryption::Rng;
use p2panda_store::SqliteStoreBuilder;

const OFFSET: i64 = 12 * 3_600_000;

fn rand32() -> [u8; 32] {
    use std::time::{SystemTime, UNIX_EPOCH};
    let n = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
    let mut out = [0u8; 32];
    out[..16].copy_from_slice(&n.to_le_bytes());
    out[16..].copy_from_slice(&(n ^ 0x9E37_79B9_7F4A_7C15).to_le_bytes());
    out
}

fn tmp(tag: &str) -> std::path::PathBuf {
    let p = std::env::temp_dir().join(format!(
        "diaswarm-wire-{tag}-{}",
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
    ));
    std::fs::create_dir_all(&p).unwrap();
    p
}

fn day(epoch: i64) -> Vec<Record> {
    (0..288i64)
        .map(|i| {
            Record::new(epoch * EPOCH_MS + i * 300_000, "cgm")
                .set("mgdl", Some((100.0 + (i % 80) as f64).into()))
        })
        .collect()
}

/// THE ENVELOPE IS AN ENVELOPE.
///
/// Segments published as operation bodies, then fetched back from the store by
/// epoch. If the recent read stays flat as the log grows, replication can be log
/// sync and [D21](../../../docs/decisions.md) survives D26.
#[tokio::test(flavor = "multi_thread")]
async fn reading_the_newest_segment_out_of_a_log_stays_flat() {
    let rng = Rng::default();
    eprintln!("  {:>10}  {:>18}  {:>14}", "days in log", "newest via ops", "whole log");

    for held in [7i64, 30, 90, 180] {
        let signing = SigningKey::from_bytes(&rand32());
        let root = tmp(&format!("log-{held}"));
        let mut vault = Vault::open(&root, OFFSET, &signing).expect("vault");
        let (mgr, _b) = Vault::key_bundle(&rng).expect("bundle");
        vault.create(mgr).expect("create");

        let store = SqliteStoreBuilder::memory().build().await.expect("store");

        for e in 0..held {
            let segment = vault.seal(20_000 + e, &day(20_000 + e)).expect("seal");
            wire::publish(&store, &signing, &segment).await.expect("publish");
        }

        let author = signing.verifying_key();

        let t = Instant::now();
        let recent = wire::segments_tail(&store, &author, 1).await.expect("read newest");
        let one = t.elapsed();
        assert_eq!(recent.len(), 1, "expected exactly the newest segment");

        let t = Instant::now();
        let all = wire::segments_from(&store, &author, i64::MIN).await.expect("read all");
        let whole = t.elapsed();
        assert_eq!(all.len() as i64, held);

        eprintln!(
            "  {held:>10}  {:>15.3}ms  {:>11.3}ms",
            one.as_secs_f64() * 1000.0,
            whole.as_secs_f64() * 1000.0
        );
    }
}
