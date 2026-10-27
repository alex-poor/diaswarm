//! "Stop sharing with everyone" must actually stop everyone — off the group's
//! live members, not a readers book that can be emptied.
//!
//! **THIS IS THE HOLE OF 2026-10-28, AS AN ASSERTION.** A subject removed every
//! "who can read" entry and a follower kept getting fresh data: the list-driven
//! stop dropped the rows but withdrew only the last reader from the group, so
//! the rest stayed members and kept the rotated secret. The fix withdraws from
//! `reader_members` — the live member set — so it is complete whatever the book
//! remembers.
//!
//! This grants two readers, confirms both are members, revokes every member,
//! seals a day AFTER the revoke, and asserts a fresh joiner for a revoked reader
//! opens the days from BEFORE the revoke but NOT the day sealed after it.

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
        "diaswarm-revokeall-{tag}-{}",
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
    ));
    std::fs::create_dir_all(&p).unwrap();
    p
}

fn day(epoch: i64) -> Vec<Record> {
    (0..4i64)
        .map(|i| Record::new(epoch * EPOCH_MS + i * 300_000, "cgm").set("mgdl", Some((100.0 + i as f64).into())))
        .collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn revoking_every_member_locks_each_out_of_new_data() {
    let rng = Rng::default();
    let subject_key = SigningKey::from_bytes(&rand32());
    let author = subject_key.verifying_key();
    let subj_dir = tmp("subject");

    let mut subject = Vault::open(&subj_dir, OFFSET, &subject_key).expect("subject");
    let (subject_mgr, subject_bundle) = Vault::key_bundle(&rng).expect("subject bundle");
    let create = subject.create(subject_mgr).expect("create");
    let store = SqliteStoreBuilder::memory().build().await.expect("store");
    wire::publish_control(&store, &subject_key, &create).await.expect("publish create");
    let keys_hex =
        encode_identity(&KeysIdentity { signer: author, bundle: subject_bundle }).expect("encode");

    // ---- two readers, each with their own identity vault ----
    let mut reader_dirs = Vec::new();
    for tag in ["a", "b"] {
        let key = SigningKey::from_bytes(&rand32());
        let own = tmp(&format!("reader-{tag}-own"));
        let (mgr, bundle) = Vault::key_bundle(&rng).expect("reader bundle");
        {
            let mut r = Vault::open(&own, OFFSET, &key).expect("reader own");
            r.create(mgr).expect("reader create");
        }
        let (welcome, _t) = subject.grant(bundle, "follow").expect("grant");
        wire::publish_control(&store, &subject_key, &welcome).await.expect("publish welcome");
        reader_dirs.push((key, own));
    }

    // ---- a day both can read, then confirm the group holds both readers ----
    let seg1 = subject.seal_delta(20_000, &day(20_000)).expect("seal day 1");
    wire::publish(&store, &subject_key, &seg1).await.expect("publish day 1");
    assert_eq!(subject.reader_members().len(), 2, "both readers must be members before the revoke");

    // ---- STOP SHARING WITH EVERYONE: revoke every current member ----
    for tag in subject.reader_members() {
        let remove = subject.revoke(tag).expect("revoke member");
        wire::publish_control(&store, &subject_key, &remove).await.expect("publish remove");
    }
    assert_eq!(subject.reader_members().len(), 0, "no reader may remain a member after revoke-all");

    // ---- a day sealed AFTER the revoke, under a secret no former reader holds ----
    let seg2 = subject.seal_delta(20_001, &day(20_001)).expect("seal day 2");
    wire::publish(&store, &subject_key, &seg2).await.expect("publish day 2");

    // ---- each revoked reader: opens the pre-revoke day, NOT the post-revoke one ----
    for (i, (key, own)) in reader_dirs.iter().enumerate() {
        let joined = tmp(&format!("reader-{i}-joined"));
        let (vault, subject_vk) =
            follow::join(own, &joined, &store, key, &keys_hex, "follow", OFFSET).await.expect("join");
        let (records, _opened, _skipped) =
            follow::read(&vault, &store, &subject_vk, 0, i64::MIN).await.expect("read");
        assert!(
            records.iter().any(|r| r.t() == 20_000 * EPOCH_MS),
            "reader {i} should still open the day from BEFORE the revoke (finished days stay)"
        );
        assert!(
            !records.iter().any(|r| r.t() == 20_001 * EPOCH_MS),
            "reader {i} MUST NOT open the day sealed AFTER revoke-all — that is the whole point"
        );
    }
}
