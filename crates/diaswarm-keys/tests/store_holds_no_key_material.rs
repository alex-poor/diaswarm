//! A `diaswarm-keys` store holds ciphertext operations and no key material.
//!
//! **THE SECURITY REVIEW'S "REOPENS IF", AS A FAST GUARD.**
//! `docs/security-review.md` acquired a real carrier's `keys.sqlite` and found
//! the four `p2panda-encryption` state tables — `groups_v1`, `key_secrets_v1`,
//! `key_registry_v1`, `spaces_v1` — empty: the store is a bare log of sealed
//! operations, and group membership and secrets live in each member's `Vault`
//! (a file tree), never in the database that crosses the network to a carrier.
//!
//! `tests/keys_replicate.rs` asserts the same thing on a carrier that received
//! its copy over real iroh — the highest-fidelity check, but slow and network-
//! bound. This is the cheap, deterministic twin: it builds the *publisher's*
//! own store — the node that actually does the sealing and granting — and shows
//! that even there the log gains no key material. If a refactor ever moved
//! encryption state into the `SqliteStore` (e.g. adopting p2panda-encryption's
//! own persistence in place of the `Vault`), those secrets would replicate
//! straight to every carrier, and this names the boundary it crossed.

use diaswarm_core::{Record, EPOCH_MS};
use diaswarm_keys::{wire, Vault};
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
        "diaswarm-nokeys-{tag}-{}",
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
    ));
    std::fs::create_dir_all(&p).unwrap();
    p
}

fn day(epoch: i64) -> Vec<Record> {
    (0..6i64)
        .map(|i| Record::new(epoch * EPOCH_MS + i * 300_000, "cgm").set("mgdl", Some((100.0 + i as f64).into())))
        .collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_published_keys_store_holds_no_key_material() {
    let rng = Rng::default();
    let subject_key = SigningKey::from_bytes(&rand32());
    let root = tmp("vault");

    // File-backed on purpose: we reopen this exact database below to count. The
    // builder is the one `diaswarm-peer` uses, so the schema is the carrier's.
    let db = tmp("store").join("keys.sqlite");
    let store = SqliteStoreBuilder::new()
        .database_url(&format!("sqlite://{}", db.display()))
        .create_database(true)
        .build()
        .await
        .unwrap();

    // The full publisher side: create the group, grant a reader, seal days —
    // every operation that ends up on a carrier, and every step that touches
    // key material. The key material lands in the Vault at `root`, not here.
    let mut subject = Vault::open(&root, OFFSET, &subject_key).unwrap();
    let (subject_mgr, _subject_bundle) = Vault::key_bundle(&rng).unwrap();
    let (_reader_mgr, reader_bundle) = Vault::key_bundle(&rng).unwrap();

    let create = subject.create(subject_mgr).unwrap();
    wire::publish_control(&store, &subject_key, &create).await.unwrap();
    let (welcome, _tag) = subject.grant(reader_bundle, "follow").unwrap();
    wire::publish_control(&store, &subject_key, &welcome).await.unwrap();

    let days = 3i64;
    for e in 0..days {
        let segment = subject.seal(30_000 + e, &day(30_000 + e)).unwrap();
        wire::publish(&store, &subject_key, &segment).await.unwrap();
    }

    // Not vacuous: the log really did fill with the sealed days and grants.
    let author = subject_key.verifying_key();
    let carried = wire::segments_tail(&store, &author, days as u64).await.unwrap();
    assert_eq!(carried.len() as i64, days, "the store never received the sealed days");

    // THE GUARD. Reopen the database a carrier would replicate and prove the
    // four p2panda-encryption state tables are empty. They exist (the builder's
    // migrations create them), so the COUNT is real, not a missing-table pass.
    let probe = sqlx::SqlitePool::connect(&format!("sqlite://{}", db.display())).await.unwrap();
    for table in ["groups_v1", "key_secrets_v1", "key_registry_v1", "spaces_v1"] {
        let rows: i64 = sqlx::query_scalar(&format!("SELECT COUNT(*) FROM {table}"))
            .fetch_one(&probe)
            .await
            .unwrap();
        assert_eq!(
            rows, 0,
            "the keys store holds {rows} row(s) in {table} — \
             encryption state has leaked into the replicated log"
        );
    }
    probe.close().await;
}
