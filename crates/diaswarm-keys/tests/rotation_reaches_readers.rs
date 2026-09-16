//! Does a rotation reach the people already reading?
//!
//! **THIS FILE EXISTS BECAUSE IT DID NOT, ON A PHONE DRIVING AN INSULIN PUMP.**
//! `Vault::rotate` mints a new group secret *and* returns the control message
//! that tells every member about it. The Android glue called it as
//! `Ok(_) => v.vault.secrets()`, throwing the message away. The secret moved,
//! nobody was told, and every segment sealed afterwards was ciphertext no
//! follower could open — while the publisher looked entirely healthy.
//!
//! Measured on 2026-09-16, three minutes after the first rotation ever to run
//! on a device: the follower's `unreadable` count climbed 0 → 1 → 2 → … one per
//! sealing pass, with readable rows falling in step. Data was arriving; only
//! the key was missing.
//!
//! The library was never wrong — `rotate` hands the message back and always
//! did. What was missing was a test that a *reader* still reads, rather than
//! one that counts secrets in the subject's own vault. So this asserts the
//! thing the outage was: not "did we rotate", but "can they still read".

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
        "diaswarm-rot-{tag}-{}",
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
    ));
    std::fs::create_dir_all(&p).unwrap();
    p
}

fn day(epoch: i64) -> Vec<Record> {
    (0..4i64)
        .map(|i| {
            Record::new(epoch * EPOCH_MS + i * 300_000, "cgm")
                .set("mgdl", Some((100.0 + i as f64).into()))
        })
        .collect()
}

/// A ROTATION NOBODY IS TOLD ABOUT IS A SILENT BLACKOUT — AND ANNOUNCING IT ENDS IT.
#[tokio::test(flavor = "multi_thread")]
async fn a_rotation_must_be_announced_or_the_reader_goes_blind() {
    let rng = Rng::default();
    let subject_key = SigningKey::from_bytes(&rand32());
    let reader_key = SigningKey::from_bytes(&rand32());
    let root = tmp("blind");

    let mut subject = Vault::open(&root, OFFSET, &subject_key).expect("subject");
    let (subject_mgr, subject_bundle) = Vault::key_bundle(&rng).expect("bundle");
    let (reader_mgr, reader_bundle) = Vault::key_bundle(&rng).expect("bundle");
    subject.create(subject_mgr).expect("create");

    let store = SqliteStoreBuilder::memory().build().await.expect("store");
    let author = subject_key.verifying_key();

    // ---- a reader, granted everything, the ordinary way ----
    let (welcome, _tag) = subject.grant(reader_bundle, "follow").expect("grant");
    wire::publish_control(&store, &subject_key, &welcome).await.expect("publish welcome");
    let welcome =
        wire::control_from(&store, &author, None).await.expect("read").pop().expect("welcome");

    let mut reader = Vault::open(&root, OFFSET, &reader_key).expect("reader");
    let registry = Vault::registry(&[(subject.subject(), subject_bundle.clone())]).expect("reg");
    reader.join(reader_mgr, registry, &subject_bundle, "follow", &welcome).expect("join");

    // ---- a day under the first secret, which they can read ----
    subject.seal(20_000, &day(20_000)).expect("seal first");
    let (got, _) = reader.read_reporting(i64::MIN).expect("read");
    assert!(got.contains_key(&20_000), "the reader could not open the day it was granted");

    // ---- the subject rotates, and the update is NOT delivered ----
    let update = subject.rotate().expect("rotate");
    subject.seal(20_001, &day(20_001)).expect("seal second");

    // **THIS IS THE OUTAGE, AS AN ASSERTION.** Nothing is wrong with the
    // subject: it rotated, it sealed, it can read its own history. The reader
    // simply cannot open anything from the new secret, and nothing anywhere
    // says so.
    let (got, skipped) = reader.read_reporting(i64::MIN).expect("read");
    assert!(
        !got.contains_key(&20_001),
        "an unannounced rotation was still readable — this test can no longer fail for its reason"
    );
    assert!(got.contains_key(&20_000), "the reader lost a day it had already been able to open");
    assert!(
        skipped.not_ours >= 1,
        "the unreadable day was not even reported as withheld, got {skipped:?}"
    );

    // ---- announce it, which is the one line the glue was missing ----
    wire::publish_control(&store, &subject_key, &update).await.expect("publish update");
    let update =
        wire::control_from(&store, &author, None).await.expect("read").pop().expect("update");
    reader.receive(&update).expect("reader takes the update");

    // ---- and the blackout is over ----
    let (got, _) = reader.read_reporting(i64::MIN).expect("read");
    assert!(
        got.contains_key(&20_001),
        "the reader still could not open the rotated day after being told about it"
    );
    assert!(got.contains_key(&20_000), "announcing the rotation cost the reader an older day");

    // And the subject was never the broken half.
    assert_eq!(
        subject.read_from(i64::MIN).expect("subject reads own").len(),
        2,
        "the subject cannot read its own two days"
    );
}
