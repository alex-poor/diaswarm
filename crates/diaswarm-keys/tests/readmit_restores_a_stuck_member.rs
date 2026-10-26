//! A human re-sharing with a member whose welcome never reached them must get
//! that reader reading again.
//!
//! **THIS IS THE SECOND HALF OF THE OUTAGE OF 2026-10-26.** The first half was
//! a reader going dark through a rotation it never applied (see
//! `follow_join_survives_rotation`). This is the half that no follower-side fix
//! can reach: the subject's member set names the reader, but no welcome that
//! reader can use ever replicated to it — its own `follow::join` reports "none
//! of the control messages welcome us". From the subject's side a second
//! `grant` returns `AlreadyGranted` and publishes nothing, so tapping "Share"
//! again does nothing and the reader stays dark forever.
//!
//! The fix is to treat a human's re-share as a re-admit: `revoke` (remove +
//! rotate) then `grant` afresh, which mints a welcome at the current secret the
//! reader CAN bootstrap from. This drives exactly that sequence at the vault
//! level — the one `grant_and_publish` runs on an attended `AlreadyGranted` —
//! and asserts a fresh joiner reads the day sealed AFTER the re-admit.

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
        "diaswarm-readmit-{tag}-{}",
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
async fn a_re_share_re_admits_a_member_and_they_read_again() {
    let rng = Rng::default();
    let subject_key = SigningKey::from_bytes(&rand32());
    let reader_key = SigningKey::from_bytes(&rand32());
    let author = subject_key.verifying_key();

    let subj_dir = tmp("subject");
    let reader_own = tmp("reader-own");

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

    // ---- grant the reader the ordinary way, then a day under that secret ----
    let (welcome, tag) = subject.grant(reader_bundle.clone(), "follow").expect("grant");
    wire::publish_control(&store, &subject_key, &welcome).await.expect("publish welcome");
    let seg1 = subject.seal_delta(20_000, &day(20_000)).expect("seal day 1");
    wire::publish(&store, &subject_key, &seg1).await.expect("publish day 1");

    // ---- a second grant is the no-op that strands them: already a member ----
    assert!(
        matches!(
            subject.grant(reader_bundle.clone(), "follow"),
            Err(diaswarm_keys::Error::AlreadyGranted(_))
        ),
        "a plain re-grant of a member must report AlreadyGranted — that is the no-op the \
         re-admit exists to get past"
    );

    // ---- the re-admit: revoke (remove + rotate) then grant afresh, exactly as
    //      `grant_and_publish` does on an attended AlreadyGranted ----
    let remove = subject.revoke(tag).expect("revoke");
    wire::publish_control(&store, &subject_key, &remove).await.expect("publish remove");
    let (welcome2, _tag2) = subject.grant(reader_bundle, "follow").expect("re-grant");
    wire::publish_control(&store, &subject_key, &welcome2).await.expect("publish welcome2");

    // ---- a day sealed AFTER the re-admit, under the current secret ----
    let seg2 = subject.seal_delta(20_001, &day(20_001)).expect("seal day 2");
    wire::publish(&store, &subject_key, &seg2).await.expect("publish day 2");

    // ---- a fresh joiner (as a phone that holds every control message is) ----
    let reader_joined = tmp("reader-joined");
    let keys_hex =
        encode_identity(&KeysIdentity { signer: author, bundle: subject_bundle }).expect("encode");
    let (vault, subject_vk) =
        follow::join(&reader_own, &reader_joined, &store, &reader_key, &keys_hex, "follow", OFFSET)
            .await
            .expect("join after re-admit");

    // ---- and it reads the day sealed after the re-admit ----
    let (records, opened, skipped) =
        follow::read(&vault, &store, &subject_vk, 0, i64::MIN).await.expect("read");

    assert!(
        records.iter().any(|r| r.t() == 20_001 * EPOCH_MS),
        "the re-admitted reader must open the day sealed after the re-admit; got {} records, \
         opened {opened}, skipped {skipped:?}",
        records.len()
    );
}
