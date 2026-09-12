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

/// WHAT COMES BACK OUT OF THE LOG MUST BE THE SEGMENT THAT WENT IN.
///
/// **THE TEST THAT WAS MISSING, AND THE BUG IT WOULD HAVE CAUGHT.**
/// `p2panda_store`'s `LogEntries<T>` is `Vec<(T, Vec<u8>)>` and reads exactly
/// like `(operation, body)`. It is not: the second half is the encoded
/// *header*, and the body is on the operation. `wire::collect` took the tuple
/// at its word, so every segment it returned carried an encoded header where
/// its ciphertext should have been.
///
/// The test above did not notice because it counts segments and times reads,
/// and a wrong ciphertext is exactly as long-lived and exactly as countable as
/// a right one. Decrypting is what tells them apart.
#[tokio::test(flavor = "multi_thread")]
async fn a_segment_out_of_a_log_still_decrypts_to_the_records_that_went_in() {
    let rng = Rng::default();
    let signing = SigningKey::from_bytes(&rand32());
    let root = tmp("roundtrip");
    let store = SqliteStoreBuilder::memory().build().await.expect("store");

    let mut vault = Vault::open(&root, OFFSET, &signing).expect("vault");
    let (mgr, _bundle) = Vault::key_bundle(&rng).expect("bundle");
    vault.create(mgr).expect("create");

    let sent = day(20_000);
    let segment = vault.seal(20_000, &sent).expect("seal");
    wire::publish(&store, &signing, &segment).await.expect("publish");

    let author = signing.verifying_key();
    let back = wire::segments_tail(&store, &author, 1).await.expect("tail");
    assert_eq!(back.len(), 1);
    assert_eq!(back[0].ciphertext, segment.ciphertext, "the log returned different bytes");

    let (records, unparseable) = vault.open_segment(&back[0]).expect("open the segment from the log");
    assert_eq!(unparseable, 0, "records came back unparseable");
    assert_eq!(records.len(), sent.len(), "the log lost records");
    assert_eq!(
        records.iter().map(|r| r.get("t").cloned()).collect::<Vec<_>>(),
        sent.iter().map(|r| r.get("t").cloned()).collect::<Vec<_>>(),
        "the records that came back are not the ones that went in"
    );
}
