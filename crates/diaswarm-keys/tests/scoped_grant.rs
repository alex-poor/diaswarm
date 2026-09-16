//! Can a grant be made to reach back only so far?
//!
//! **THIS IS THE SPIKE [D29](../../../docs/decisions.md) ASKED FOR, AND IT
//! EXISTS BECAUSE THAT DECISION ENDS BY SAYING IT HAD NOT BEEN RUN.** The words
//! were: *"This is read from the source. Nothing has been built, compiled or
//! measured, and on the evidence of this repository's last three days that
//! distinction is the whole difference between a finding and a result."*
//!
//! The finding was that a time-scoped grant is reachable on pinned
//! p2panda-encryption 0.7.1. That much holds. The route does not: D29 proposed
//! dropping to `Dcgka::add`, which takes the bundle as a parameter — but
//! rebuilding what `EncryptionGroup::add` does around it needs `process_local`,
//! and that is private. The way through is one layer higher and simpler.
//! `GroupState::secrets` is a public field and `update_secrets` is a public
//! setter, so the bundle a joiner receives is whatever is in that field when
//! `add` runs.
//!
//! What this file asserts is the only thing that matters about that: a reader
//! granted from a narrowed bundle can open what came after the cut and cannot
//! open what came before it.

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
        "diaswarm-scoped-{tag}-{}",
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

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

/// A SCOPED GRANT OPENS WHAT CAME AFTER THE CUT, AND NOTHING BEFORE IT.
///
/// The subject seals a day, rotates the secret, seals another, then grants a
/// reader with `since` set between the two rotations. The reader gets one day,
/// not two — which is the sentence a clinician-facing "share the last 90 days"
/// has to be able to mean.
#[tokio::test(flavor = "multi_thread")]
async fn a_grant_can_be_scoped_to_secrets_made_after_a_cutoff() {
    let rng = Rng::default();
    let subject_key = SigningKey::from_bytes(&rand32());
    let reader_key = SigningKey::from_bytes(&rand32());
    let root = tmp("scoped");

    let mut subject = Vault::open(&root, OFFSET, &subject_key).expect("subject");
    let (subject_mgr, subject_bundle) = Vault::key_bundle(&rng).expect("bundle");
    let (reader_mgr, reader_bundle) = Vault::key_bundle(&rng).expect("bundle");
    subject.create(subject_mgr).expect("create");

    // ---- the old day, under the first secret ----
    subject.seal(20_000, &day(20_000)).expect("seal old");

    // **THE CUT HAS TO BE A WHOLE SECOND LATER.** `GroupSecret`'s timestamp is
    // UNIX *seconds* taken from `SystemTime::now()`, so two secrets minted in
    // the same second are indistinguishable to any filter over them. That is a
    // real limit on how finely this can scope, not a quirk of the test.
    std::thread::sleep(std::time::Duration::from_millis(1_100));
    let cutoff = now_secs();

    subject.rotate().expect("rotate");
    assert!(subject.secrets() >= 2, "rotation did not add a secret");

    // ---- the new day, under the second ----
    subject.seal(20_001, &day(20_001)).expect("seal new");

    // ---- the grant, scoped ----
    let (welcome, _tag) =
        subject.grant_since(reader_bundle, "follow", cutoff).expect("scoped grant");

    // Through the wire, because that is the only way to get an `Authentic` and
    // the only path a real reader has.
    let store = SqliteStoreBuilder::memory().build().await.expect("store");
    wire::publish_control(&store, &subject_key, &welcome).await.expect("publish");
    let mut arrived = wire::control_from(&store, &subject_key.verifying_key(), None)
        .await
        .expect("read control");
    let welcome = arrived.pop().expect("welcome");

    // **THE SAME ROOT, SO THE READER CAN SEE THE SEGMENTS.** Counting secrets
    // is a proxy; what is actually claimed is what can be opened, and that
    // needs the ciphertext in front of it.
    let mut reader = Vault::open(&root, OFFSET, &reader_key).expect("reader");
    let registry =
        Vault::registry(&[(subject.subject(), subject_bundle.clone())]).expect("registry");
    reader.join(reader_mgr, registry, &subject_bundle, "follow", &welcome).expect("join");

    // The reader holds exactly one secret: the one minted after the cut.
    assert_eq!(
        reader.secrets(),
        1,
        "a scoped grant handed over {} secrets, not 1",
        reader.secrets()
    );

    // ---- and this is the claim itself ----
    let (got, skipped) = reader.read_reporting(i64::MIN).expect("reader reads");
    assert!(got.contains_key(&20_001), "the scoped reader could not open the day it was granted");
    assert!(
        !got.contains_key(&20_000),
        "the scoped reader opened the day BEFORE its cutoff — the narrowing did nothing"
    );
    assert_eq!(
        skipped.not_ours, 1,
        "expected exactly one day withheld, got {:?}",
        skipped
    );

    // ---- and the subject did not lose anything by granting ----
    assert!(
        subject.secrets() >= 2,
        "granting narrowly left the subject holding {} secrets",
        subject.secrets()
    );
    let mine = subject.read_from(i64::MIN).expect("subject reads own history");
    assert_eq!(mine.len(), 2, "the subject can no longer read its own two days");
}

/// AND AN UNSCOPED GRANT STILL REACHES BACK OVER EVERYTHING.
///
/// The control. If this passes and the one above does not, the narrowing did
/// nothing; if both pass, `since` is what made the difference.
#[tokio::test(flavor = "multi_thread")]
async fn an_unscoped_grant_still_hands_over_the_whole_history() {
    let rng = Rng::default();
    let subject_key = SigningKey::from_bytes(&rand32());
    let reader_key = SigningKey::from_bytes(&rand32());
    let root = tmp("unscoped");

    let mut subject = Vault::open(&root, OFFSET, &subject_key).expect("subject");
    let (subject_mgr, subject_bundle) = Vault::key_bundle(&rng).expect("bundle");
    let (reader_mgr, reader_bundle) = Vault::key_bundle(&rng).expect("bundle");
    subject.create(subject_mgr).expect("create");

    subject.seal(20_000, &day(20_000)).expect("seal old");
    std::thread::sleep(std::time::Duration::from_millis(1_100));
    subject.rotate().expect("rotate");
    subject.seal(20_001, &day(20_001)).expect("seal new");

    let (welcome, _tag) = subject.grant(reader_bundle, "follow").expect("grant");

    let store = SqliteStoreBuilder::memory().build().await.expect("store");
    wire::publish_control(&store, &subject_key, &welcome).await.expect("publish");
    let mut arrived = wire::control_from(&store, &subject_key.verifying_key(), None)
        .await
        .expect("read control");
    let welcome = arrived.pop().expect("welcome");

    let mut reader = Vault::open(&root, OFFSET, &reader_key).expect("reader");
    let registry =
        Vault::registry(&[(subject.subject(), subject_bundle.clone())]).expect("registry");
    reader.join(reader_mgr, registry, &subject_bundle, "follow", &welcome).expect("join");

    assert!(
        reader.secrets() >= 2,
        "an unscoped grant handed over only {} secret(s) — the control is broken",
        reader.secrets()
    );
    let (got, skipped) = reader.read_reporting(i64::MIN).expect("reader reads");
    assert_eq!(got.len(), 2, "an unscoped reader should see both days, saw {}", got.len());
    assert_eq!(skipped.not_ours, 0, "an unscoped reader was refused a day: {:?}", skipped);
}
