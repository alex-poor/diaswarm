//! Do `diaswarm-core` and `diaswarm-keys` give a reader the same records?
//!
//! **THE TEST D20 SAID WOULD JUSTIFY DELETING `vault.rs`**, asked of
//! [D26](../../../docs/decisions.md)'s vault instead of the spaces one.
//! Everything else measured about `diaswarm-keys` is a property — flat reads,
//! revocation cutting forward. This asks the duller and more important
//! question: put a real person's history through both implementations and does
//! the same data come out?
//!
//! A migration that quietly drops a bolus is worse than no migration, and the
//! hand-rolled vault has dropped records before — 4,276 of them, from the only
//! copy a subject had.
//!
//! **BY DEFAULT IT RUNS ON SYNTHETIC RECORDS**, so it works for anyone with no
//! data and no phone. Point it at a real canonical stream to run it on real
//! history — `tools/canon.py <aaps.db> -o records.ndjson` produces one:
//!
//! ```sh
//! DIASWARM_RECORDS=/path/to/records.ndjson cargo test --test differential
//! ```
//!
//! The file is never committed and nothing here writes it anywhere but a temp
//! directory. It is somebody's glucose history.

use std::collections::BTreeMap;
use std::path::PathBuf;

use diaswarm_core::vault::{Identity, Vault as CoreVault};
use diaswarm_core::{EPOCH_MS, Record, epoch_of, kind};
use diaswarm_keys::{Vault as KeysVault, wire};
use p2panda_store::{SqliteStore, SqliteStoreBuilder};
use p2panda_core::SigningKey;
use p2panda_encryption::Rng;

const OFFSET: i64 = 12 * 3_600_000;

fn tmp(tag: &str) -> PathBuf {
    let p = std::env::temp_dir().join(format!(
        "diaswarm-keysdiff-{tag}-{}",
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
    ));
    std::fs::create_dir_all(&p).unwrap();
    p
}

fn rand32() -> [u8; 32] {
    use std::time::{SystemTime, UNIX_EPOCH};
    let n = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
    let mut out = [0u8; 32];
    out[..16].copy_from_slice(&n.to_le_bytes());
    out[16..].copy_from_slice(&(n ^ 0x9E37_79B9_7F4A_7C15).to_le_bytes());
    out
}

fn records() -> (Vec<Record>, &'static str) {
    if let Ok(path) = std::env::var("DIASWARM_RECORDS") {
        let text = std::fs::read_to_string(&path).expect("DIASWARM_RECORDS is not readable");
        let parsed: Vec<Record> = text
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .filter_map(|l| Record::from_json(l).ok())
            // Each implementation emits the stream header for itself, so
            // counting them would compare bookkeeping rather than data.
            .filter(|r| r.kind() != kind::META)
            .collect();
        assert!(!parsed.is_empty(), "{path} parsed to no records");
        return (parsed, "real");
    }

    let mut out = Vec::new();
    for day in 0..12i64 {
        let base = (22_000 + day) * EPOCH_MS;
        for slot in 0..24i64 {
            out.push(
                Record::new(base + slot * 3_600_000, "cgm")
                    .set("mgdl", Some((90.0 + (slot as f64 * 3.1) % 140.0).into())),
            );
        }
        out.push(Record::new(base + 7_200_000, "bolus").set("units", Some(2.5.into())));
        out.push(Record::new(base + 7_200_000, "carbs").set("grams", Some(40.0.into())));
    }
    (out, "synthetic")
}

fn canonical(records: &[Record]) -> Vec<String> {
    let mut v: Vec<String> =
        records.iter().filter(|r| r.kind() != kind::META).map(|r| r.to_canonical_json()).collect();
    v.sort();
    v.dedup();
    v
}

/// THE SAME RECORDS COME OUT OF BOTH.
#[tokio::test(flavor = "multi_thread")]
async fn both_vaults_give_a_reader_the_same_history() {
    let (all, source) = records();
    let mut by_epoch: BTreeMap<i64, Vec<Record>> = BTreeMap::new();
    for r in &all {
        by_epoch.entry(epoch_of(r.t(), OFFSET)).or_default().push(r.clone());
    }
    if let Ok(cap) = std::env::var("DIASWARM_EPOCHS") {
        let cap: usize = cap.parse().expect("DIASWARM_EPOCHS is a number");
        while by_epoch.len() > cap {
            let last = *by_epoch.keys().next_back().expect("non-empty");
            by_epoch.remove(&last);
        }
    }
    eprintln!(
        "  {source}: {} records over {} epochs",
        by_epoch.values().map(Vec::len).sum::<usize>(),
        by_epoch.len()
    );

    // ---- the shipping vault ----
    let core_root = tmp("core");
    let subject = Identity::generate();
    let reader = Identity::generate();
    let core = CoreVault::create(&core_root, &subject, OFFSET).expect("core vault");
    for (epoch, records) in &by_epoch {
        core.seal(*epoch, records).expect("core seal");
    }
    let reader_pub: [u8; 32] = x25519_dalek::PublicKey::from(&reader.encryption).to_bytes();
    core.record_grant(&subject, &reader_pub, "follow", "grant", 0).expect("core grant");
    core.publish_wraps(&subject, &reader_pub, "follow").expect("core wraps");
    let from_core: Vec<Record> =
        core.read_as(&reader, "follow").expect("core read").into_values().flatten().collect();

    // ---- D26's vault ----
    let rng = Rng::default();
    let keys_root = tmp("keys");
    let subject_key = SigningKey::from_bytes(&rand32());
    let reader_key = SigningKey::from_bytes(&rand32());
    let mut keys = KeysVault::open(&keys_root, OFFSET, &subject_key).expect("keys vault");
    let (subject_mgr, subject_bundle) = KeysVault::key_bundle(&rng).expect("bundle");
    let (reader_mgr, reader_bundle) = KeysVault::key_bundle(&rng).expect("bundle");
    keys.create(subject_mgr).expect("create");
    let (welcome, _tag) = keys.grant(reader_bundle, "follow").expect("grant");
    for (epoch, records) in &by_epoch {
        keys.seal(*epoch, records).expect("keys seal");
    }
    let mut keys_reader = KeysVault::open(&keys_root, OFFSET, &reader_key).expect("reader vault");
    let registry = KeysVault::registry(&[(keys.subject(), subject_bundle.clone())]).expect("registry");
    let store = SqliteStoreBuilder::memory().build().await.expect("store");
    let welcome = deliver(&store, &subject_key, &welcome).await;
    keys_reader.join(reader_mgr, registry, &subject_bundle, "follow", &welcome).expect("join");
    let from_keys: Vec<Record> =
        keys_reader.read_from(i64::MIN).expect("keys read").into_values().flatten().collect();

    // ---- and they agree ----
    let a = canonical(&from_core);
    let b = canonical(&from_keys);
    eprintln!("  core {} records · keys {} records", a.len(), b.len());
    assert!(!a.is_empty(), "the shipping vault read nothing");
    assert_eq!(a.len(), b.len(), "the two vaults returned different counts");
    assert_eq!(a, b, "the two vaults disagreed about the records themselves");
}

/// Put a control message on the wire and take it off again.
///
/// **THE ONLY WAY INTO A VAULT NOW, AND THAT IS THE POINT.** `join` and
/// `receive` take an `Authentic`, and the only constructor of one is
/// `wire::open_control`. A test that wants to hand a welcome over has to sign
/// it and verify it exactly as the network would, which is what this does.
async fn deliver(
    store: &SqliteStore,
    signing: &SigningKey,
    message: &diaswarm_keys::group::Message,
) -> diaswarm_keys::Authentic {
    wire::publish_control(store, signing, message).await.expect("publish control");
    wire::control_from(store, &signing.verifying_key(), None)
        .await
        .expect("read control")
        .pop()
        .expect("a control message came back")
}
