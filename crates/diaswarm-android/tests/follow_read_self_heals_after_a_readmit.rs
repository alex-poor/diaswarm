//! `follow_read` must recover a persisted follower after the subject re-admits.
//!
//! This is the orchestration test for the outage of 2026-10-26. The library
//! primitives are proven in `diaswarm-keys/tests/readmit_reaches_a_persisted_follower`
//! (receive-replay strands a persisted vault; a clean re-join recovers it). This
//! drives the actual production entry point — `follow_read` — over ONE persisted
//! `joined_dir`, exactly as the follower phone calls it every refresh:
//!
//!   1. grant + a day, then `follow_read` → it joins, persists, and reads day 1;
//!   2. the subject re-admits (revoke + re-grant) and seals day 2;
//!   3. `follow_read` on the SAME dir again → it must read day 2.
//!
//! Before the fix, step 3 took the "already welcomed → receive-replay" branch,
//! applied the follower's own removal, could not bootstrap from the new welcome,
//! and returned `opened 0` forever. The fix makes it detect the strand (welcomed
//! yet opening nothing while unreadable segments exist) and re-join cleanly.

use diaswarm_core::{EPOCH_MS, Record};
use diaswarm_keys::{encode_identity, wire, KeysIdentity, Rng, SqliteStoreBuilder, Vault};
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
        "diaswarm-followread-{tag}-{}",
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

/// Parse the `opened` count out of `follow_read`'s `ok <opened> <unreadable>\n...`.
fn opened_of(reply: &str) -> usize {
    let first = reply.lines().next().unwrap_or("");
    assert!(first.starts_with("ok "), "follow_read failed: {first}");
    first.split_whitespace().nth(1).unwrap().parse().unwrap()
}

#[test]
fn follow_read_recovers_a_persisted_follower_across_a_readmit() {
    let runtime = tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap();
    let rt = runtime.handle();
    let rng = Rng::default();

    let subject_key = SigningKey::from_bytes(&rand32());
    let reader_key = SigningKey::from_bytes(&rand32());
    let author = subject_key.verifying_key();

    let subj_dir = tmp("subject");
    let reader_own = tmp("reader-own");
    let reader_joined = tmp("reader-joined");

    let store = rt.block_on(async { SqliteStoreBuilder::memory().build().await.unwrap() });

    // ---- subject + group ----
    let mut subject = Vault::open(&subj_dir, OFFSET, &subject_key).unwrap();
    let (subject_mgr, subject_bundle) = Vault::key_bundle(&rng).unwrap();
    let create = subject.create(subject_mgr).unwrap();
    rt.block_on(wire::publish_control(&store, &subject_key, &create)).unwrap();

    // ---- reader's own identity vault ----
    let (reader_mgr, reader_bundle) = Vault::key_bundle(&rng).unwrap();
    {
        let mut r = Vault::open(&reader_own, OFFSET, &reader_key).unwrap();
        r.create(reader_mgr).unwrap();
    }
    let keys_hex =
        encode_identity(&KeysIdentity { signer: author, bundle: subject_bundle }).unwrap();

    // ---- grant + day 1, then follow_read joins/persists and reads it ----
    let (welcome, tag) = subject.grant(reader_bundle.clone(), "follow").unwrap();
    rt.block_on(wire::publish_control(&store, &subject_key, &welcome)).unwrap();
    let seg1 = subject.seal_delta(20_000, &day(20_000)).unwrap();
    rt.block_on(wire::publish(&store, &subject_key, &seg1)).unwrap();

    let first = diaswarm_android::follow_read(
        &reader_own, &reader_joined, &store, rt, &reader_key, &keys_hex, "follow", 0, i64::MIN, OFFSET,
    );
    assert!(opened_of(&first) > 0, "first follow_read must open day 1; got: {}", first.lines().next().unwrap());
    assert!(reader_joined.join("group.cbor").exists(), "the follower must have persisted its join");

    // ---- the subject RE-ADMITS and seals a day under the new secret ----
    let remove = subject.revoke(tag).unwrap();
    rt.block_on(wire::publish_control(&store, &subject_key, &remove)).unwrap();
    let (welcome2, _tag2) = subject.grant(reader_bundle, "follow").unwrap();
    rt.block_on(wire::publish_control(&store, &subject_key, &welcome2)).unwrap();
    let seg2 = subject.seal_delta(20_001, &day(20_001)).unwrap();
    rt.block_on(wire::publish(&store, &subject_key, &seg2)).unwrap();

    // ---- follow_read on the SAME persisted dir must recover and read day 2 ----
    let after = diaswarm_android::follow_read(
        &reader_own, &reader_joined, &store, rt, &reader_key, &keys_hex, "follow", 0, i64::MIN, OFFSET,
    );
    assert!(
        opened_of(&after) > 0,
        "after a re-admit, follow_read on the persisted follower must recover and open the new \
         day — a stuck 0 here is the live outage. got: {}",
        after.lines().next().unwrap()
    );
    assert!(
        after.contains(&(20_001 * EPOCH_MS).to_string()),
        "the recovered follower must specifically open the day sealed AFTER the re-admit"
    );
}
