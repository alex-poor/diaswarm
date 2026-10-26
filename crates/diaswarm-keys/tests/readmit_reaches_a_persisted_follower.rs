//! A re-admit must reach a follower that has ALREADY joined and persisted.
//!
//! **THIS IS THE THIRD FACE OF THE OUTAGE OF 2026-10-26, AND THE ONE THE OTHER
//! TWO TESTS MISS.** `readmit_restores_a_stuck_member` proves the subject mints
//! a usable welcome on a re-share, and `follow_join_survives_rotation` proves a
//! FRESH `follow::join` reads past a rotation. Both join into an empty
//! directory. A live follower does not: it joined once, left `group.cbor`
//! behind, and `follow_read` then takes the "already welcomed" branch — it
//! replays the control log onto the PERSISTED vault with `receive` and never
//! re-runs `join`.
//!
//! That branch cannot consume a re-admit. A re-admit is `remove` (the follower
//! receives its OWN removal) then a fresh `grant` (a new welcome at the new
//! secret). Replaying `receive` over the persisted vault applies the removal —
//! the follower goes dark — and the new welcome does not bootstrap it back,
//! because `receive` is not the welcome path. So the follower stays dark across
//! a re-share forever, which is exactly what a phone showed live: the subject
//! re-shared, the welcome replicated, and `opened` stayed 0.
//!
//! This drives BOTH paths over one persisted directory:
//!   1. the production "already welcomed → receive-replay" path, and asserts it
//!      does NOT recover the day sealed after the re-admit (the live bug), then
//!   2. a clean re-join (wipe the dir, `follow::join` afresh) and asserts it
//!      DOES — which is the fix `follow_read` must fall back to when a welcomed
//!      vault has been revoked out from under itself.

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
        "diaswarm-persisted-{tag}-{}",
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
async fn a_re_admit_reaches_a_follower_that_already_joined() {
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

    // ---- the reader's OWN vault (its identity, which a re-admit re-wraps to) ----
    let (reader_mgr, reader_bundle) = Vault::key_bundle(&rng).expect("reader bundle");
    {
        let mut r = Vault::open(&reader_own, OFFSET, &reader_key).expect("reader own");
        r.create(reader_mgr).expect("reader create");
    }
    let keys_hex =
        encode_identity(&KeysIdentity { signer: author, bundle: subject_bundle }).expect("encode");

    // ---- grant, seal a day, and let the follower JOIN AND PERSIST ----
    let (welcome, tag) = subject.grant(reader_bundle.clone(), "follow").expect("grant");
    wire::publish_control(&store, &subject_key, &welcome).await.expect("publish welcome");
    let seg1 = subject.seal_delta(20_000, &day(20_000)).expect("seal day 1");
    wire::publish(&store, &subject_key, &seg1).await.expect("publish day 1");

    let (vault, subject_vk) =
        follow::join(&reader_own, &reader_joined, &store, &reader_key, &keys_hex, "follow", OFFSET)
            .await
            .expect("first join");
    let (pre, _, _) = follow::read(&vault, &store, &subject_vk, 0, i64::MIN).await.expect("pre read");
    assert!(
        pre.iter().any(|r| r.t() == 20_000 * EPOCH_MS),
        "the follower must read the first day it was granted before anything is re-admitted"
    );
    drop(vault); // group.cbor is now on disk in reader_joined — a LIVE follower's state

    // ---- the subject re-admits (revoke = remove + rotate, then grant afresh) ----
    let remove = subject.revoke(tag).expect("revoke");
    wire::publish_control(&store, &subject_key, &remove).await.expect("publish remove");
    let (welcome2, _tag2) = subject.grant(reader_bundle, "follow").expect("re-grant");
    wire::publish_control(&store, &subject_key, &welcome2).await.expect("publish welcome2");
    let seg2 = subject.seal_delta(20_001, &day(20_001)).expect("seal day 2");
    wire::publish(&store, &subject_key, &seg2).await.expect("publish day 2");

    // ---- (1) the PRODUCTION "already welcomed → receive-replay" path ----
    //      Exactly what `follow_read` does for a vault that is already welcomed:
    //      reopen the persisted dir and replay the control log with `receive`.
    let replay_opened = {
        let mut v = Vault::open(&reader_joined, OFFSET, &reader_key).expect("reopen persisted");
        assert!(v.is_welcomed(), "the persisted follower is welcomed — that is why follow_read \
                                  takes the receive-replay branch and not a fresh join");
        let control = wire::control_from(&store, &author, None).await.expect("control");
        for message in &control {
            let _ = v.receive(message);
        }
        let (records, opened, _skipped) =
            follow::read(&v, &store, &subject_vk, 0, i64::MIN).await.expect("replay read");
        eprintln!("receive-replay path: opened {opened}, {} records", records.len());
        records.iter().any(|r| r.t() == 20_001 * EPOCH_MS)
    };
    assert!(
        !replay_opened,
        "REPRODUCTION: the receive-replay path is expected NOT to recover the post-re-admit day \
         — that is the live bug. If this now passes, receive CAN re-admit and the fix is elsewhere."
    );

    // ---- (2) the FIX: wipe the joined dir and re-join cleanly ----
    std::fs::remove_dir_all(&reader_joined).ok();
    std::fs::create_dir_all(&reader_joined).ok();
    let (v2, subject_vk2) =
        follow::join(&reader_own, &reader_joined, &store, &reader_key, &keys_hex, "follow", OFFSET)
            .await
            .expect("clean re-join after re-admit");
    let (records2, opened2, skipped2) =
        follow::read(&v2, &store, &subject_vk2, 0, i64::MIN).await.expect("read after re-join");
    assert!(
        records2.iter().any(|r| r.t() == 20_001 * EPOCH_MS),
        "a clean re-join must open the day sealed after the re-admit; got {} records, opened \
         {opened2}, skipped {skipped2:?}",
        records2.len()
    );
}
