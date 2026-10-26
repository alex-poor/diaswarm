//! A reader that joined through `follow::join` must still read after a rotation.
//!
//! **THIS IS THE OUTAGE OF 2026-10-26, AS AN ASSERTION.** A follower was granted
//! at 19:57 and read fine. At 23:16 the subject's `rotateIfDue` rotated the group
//! secret. From then on every segment was sealed under the new secret, and the
//! follower went dark — "waiting", `key missing`, `not_ours` climbing — for the
//! rest of the night.
//!
//! The library was not missing the secret: `rotate` publishes the update and it
//! replicated to the follower (its control log held every message). The gap was
//! in `follow::join`, which applied the ONE message that welcomed the reader and
//! returned, never applying the rotation that came after — so the joined vault
//! stayed frozen on the epoch of its welcome. The sibling test
//! `rotation_reaches_readers` never caught it because it applied the update by
//! hand with `receive`; the production path that a phone actually runs did not.
//!
//! So this drives the PRODUCTION path — `follow::join` then `follow::read` — and
//! asserts the reader opens the day sealed AFTER the rotation, not just the day
//! before it.

use diaswarm_core::{EPOCH_MS, Record};
use diaswarm_keys::{encode_identity, follow, wire, KeysIdentity, Rng, SqliteStoreBuilder, Vault};
use p2panda_core::SigningKey;

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
        "diaswarm-fjr-{tag}-{}",
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

#[tokio::test(flavor = "multi_thread")]
async fn follow_join_survives_a_rotation() {
    let rng = Rng::default();
    let subject_key = SigningKey::from_bytes(&rand32());
    let reader_key = SigningKey::from_bytes(&rand32());
    let author = subject_key.verifying_key();

    let subj_dir = tmp("subject");
    let reader_own = tmp("reader-own");
    let reader_joined = tmp("reader-joined");

    // ---- the subject, with its group ----
    let mut subject = Vault::open(&subj_dir, OFFSET, &subject_key).expect("subject");
    let (subject_mgr, subject_bundle) = Vault::key_bundle(&rng).expect("subject bundle");
    let create = subject.create(subject_mgr).expect("create");
    let store = SqliteStoreBuilder::memory().build().await.expect("store");
    wire::publish_control(&store, &subject_key, &create).await.expect("publish create");

    // ---- the reader's OWN vault, so `follow::join` can read its manager ----
    let (reader_mgr, reader_bundle) = Vault::key_bundle(&rng).expect("reader bundle");
    {
        let mut r = Vault::open(&reader_own, OFFSET, &reader_key).expect("reader own");
        r.create(reader_mgr).expect("reader create");
    }

    // ---- grant the reader, the ordinary way ----
    let (welcome, _tag) = subject.grant(reader_bundle, "follow").expect("grant");
    wire::publish_control(&store, &subject_key, &welcome).await.expect("publish welcome");

    // ---- a day under the first secret ----
    let seg1 = subject.seal_delta(20_000, &day(20_000)).expect("seal day 1");
    wire::publish(&store, &subject_key, &seg1).await.expect("publish day 1");

    // ---- the subject ROTATES (as `rotateIfDue` does), publishes the update,
    //      and seals a second day under the NEW secret ----
    let update = subject.rotate().expect("rotate");
    wire::publish_control(&store, &subject_key, &update).await.expect("publish update");
    let seg2 = subject.seal_delta(20_001, &day(20_001)).expect("seal day 2");
    wire::publish(&store, &subject_key, &seg2).await.expect("publish day 2");

    // ---- the reader joins through the PRODUCTION path, holding every control
    //      message — welcome AND the rotation that followed it ----
    let keys_hex =
        encode_identity(&KeysIdentity { signer: author, bundle: subject_bundle }).expect("encode");
    let (vault, subject_vk) =
        follow::join(&reader_own, &reader_joined, &store, &reader_key, &keys_hex, "follow", OFFSET)
            .await
            .expect("join");

    // ---- and it reads BOTH days, not just the one before the rotation ----
    let (records, opened, skipped) =
        follow::read(&vault, &store, &subject_vk, 0, i64::MIN).await.expect("read");

    assert_eq!(
        records.len(),
        8,
        "the reader should open both days (4 + 4); a frozen-at-welcome join opens only the \
         pre-rotation day and reports the rest as not-ours. got {} records, opened {opened}, \
         skipped {skipped:?}",
        records.len()
    );
    assert_eq!(
        skipped.not_ours, 0,
        "nothing was actually outside this reader's window — a non-zero not_ours here is the \
         rotation blackout: segments sealed under a secret the join never applied. got {skipped:?}"
    );
}
