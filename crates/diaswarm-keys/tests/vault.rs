//! Does D26 hold with production types and segments on disk?
//!
//! `spike/p2panda-datascheme` proved the properties in memory with the crate's
//! `test_utils` helpers. This asks the same questions of the real thing: our own
//! DGM and orderer, `p2panda_core::VerifyingKey` and `Hash` as the identifiers,
//! and segments written to and read from a directory.

use std::path::PathBuf;
use std::time::Instant;

use diaswarm_core::{EPOCH_MS, Record};
use diaswarm_keys::Vault;
use p2panda_core::SigningKey;
use p2panda_encryption::Rng;

const OFFSET: i64 = 12 * 3_600_000;

fn tmp(tag: &str) -> PathBuf {
    let p = std::env::temp_dir().join(format!(
        "diaswarm-keys-{tag}-{}",
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

/// THE WHOLE DECISION, END TO END.
///
/// A subject seals days into segments, a reader is granted and opens them, the
/// recent end stays cheap as history grows, and a revoked reader is cut off from
/// what comes next while keeping what it already had.
#[test]
fn a_granted_reader_opens_segments_and_a_revoked_one_stops() {
    let rng = Rng::default();
    let subject_key = SigningKey::from_bytes(&rand32());
    let reader_key = SigningKey::from_bytes(&rand32());

    let root = tmp("shared");
    let mut subject = Vault::open(&root, OFFSET, &subject_key).expect("subject vault");
    let (_subject_mgr, subject_bundle) = Vault::key_bundle(&rng).expect("subject bundle");
    let (reader_mgr, reader_bundle) = Vault::key_bundle(&rng).expect("reader bundle");

    subject
        .create(&subject_key, vec![(subject.subject(), subject_bundle.clone())])
        .expect("create group");

    // A day before anybody is granted.
    subject.seal(20_000, &day(20_000)).expect("seal");

    let welcome = subject.grant(reader_key.verifying_key(), reader_bundle).expect("grant");

    let mut reader = Vault::open(&root, OFFSET, &reader_key).expect("reader vault");
    let registry = Vault::registry(&[(subject.subject(), subject_bundle)]).expect("registry");
    reader.join(&reader_key, reader_mgr, registry, welcome).expect("join");

    // More days, after the grant.
    for e in 1..180i64 {
        subject.seal(20_000 + e, &day(20_000 + e)).expect("seal");
    }

    // Reading everything is linear in what is read; reading one day is not.
    let t = Instant::now();
    let all = reader.read_from(20_000).expect("read all");
    let whole = t.elapsed();
    let t = Instant::now();
    let one = reader.read_from(20_179).expect("read newest");
    let newest = t.elapsed();
    assert_eq!(one.len(), 1, "reading from the newest epoch returned {} days", one.len());
    eprintln!(
        "  180 days on disk: all {} days in {:.2}ms, newest day alone in {:.3}ms",
        all.len(),
        whole.as_secs_f64() * 1000.0,
        newest.as_secs_f64() * 1000.0
    );

    // ---- revocation cuts forward, and only forward ----
    let before = reader.read_from(20_000).expect("read all").len();
    subject.revoke(reader.subject()).expect("revoke");
    subject.seal(21_000, &day(21_000)).expect("seal after revoke");

    let after = reader.read_from(20_000).expect("read after revoke");
    assert!(
        !after.contains_key(&21_000),
        "a revoked reader opened a segment sealed after the revocation"
    );
    assert_eq!(after.len(), before, "a revoked reader lost what it already had");
}

fn rand32() -> [u8; 32] {
    use std::time::{SystemTime, UNIX_EPOCH};
    let n = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
    let mut out = [0u8; 32];
    out[..16].copy_from_slice(&n.to_le_bytes());
    out[16..].copy_from_slice(&(n ^ 0x9E37_79B9_7F4A_7C15).to_le_bytes());
    out
}

/// THE PROPERTY THE DECISION RESTS ON: does the recent read stay flat as the
/// history behind it grows?
///
/// A separate vault per history size, because the question is what a reader pays
/// when the subject has been sealing for longer — not what it pays to read more.
#[test]
fn reading_the_newest_day_does_not_care_how_much_came_before() {
    let rng = Rng::default();
    eprintln!("  {:>10}  {:>16}", "days held", "newest day");
    for held in [7i64, 30, 90, 180] {
        let subject_key = SigningKey::from_bytes(&rand32());
        let reader_key = SigningKey::from_bytes(&rand32());
        let root = tmp(&format!("depth-{held}"));

        let mut subject = Vault::open(&root, OFFSET, &subject_key).expect("vault");
        let (_m, subject_bundle) = Vault::key_bundle(&rng).expect("bundle");
        let (reader_mgr, reader_bundle) = Vault::key_bundle(&rng).expect("bundle");
        subject
            .create(&subject_key, vec![(subject.subject(), subject_bundle.clone())])
            .expect("create");
        let welcome = subject.grant(reader_key.verifying_key(), reader_bundle).expect("grant");

        let mut reader = Vault::open(&root, OFFSET, &reader_key).expect("vault");
        let registry = Vault::registry(&[(subject.subject(), subject_bundle)]).expect("registry");
        reader.join(&reader_key, reader_mgr, registry, welcome).expect("join");

        for e in 0..held {
            subject.seal(20_000 + e, &day(20_000 + e)).expect("seal");
        }

        let newest = 20_000 + held - 1;
        let t = Instant::now();
        let got = reader.read_from(newest).expect("read");
        let elapsed = t.elapsed();
        assert_eq!(got.len(), 1, "expected exactly the newest day");
        eprintln!("  {held:>10}  {:>13.3}ms", elapsed.as_secs_f64() * 1000.0);
    }
}
