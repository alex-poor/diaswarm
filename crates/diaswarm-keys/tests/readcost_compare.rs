//! What does a read cost in each vault, and does it grow with history?
//!
//! **THE MEASUREMENT THE BATTERY INVESTIGATION ASKED FOR.** A profile of the
//! follower on 2026-09-17 put 64% of the process's cycles in
//! `libdiaswarm_android.so` and a further 28% in the allocator, with
//! curve25519 field arithmetic the single largest item at ~17% of everything.
//! The source of that is [`diaswarm_core::vault::Vault::read_as_from`]: it
//! runs one X25519 Diffie-Hellman per segment per read (`unwrap`), decrypts
//! every segment, and then re-serialises every record to canonical JSON to
//! deduplicate — none of it cached between reads.
//!
//! D26's vault should not do any of that. A segment names the secret that
//! opens it, the secret comes out of `p2panda-encryption`'s `SecretBundle` by
//! id, and the decrypt is symmetric:
//!
//! ```ignore
//! let secret = state.secrets.get(&segment.secret_id)?;
//! let plain  = decrypt_data(&segment.ciphertext, secret, segment.nonce)?;
//! ```
//!
//! That is a hash lookup where the other is a scalar multiplication. This
//! measures whether that difference is real and whether either curve bends
//! upward with history held — the question `readcost.rs` asks of the shipping
//! vault alone, asked of both so the migration has a number attached.
//!
//! It also measures the cost `follow.rs` warns about in its own comment — that
//! the keys reader fetches segment supersets it then discards, and that this
//! "is the thing to revisit first if replication gets expensive".
//!
//! **BY DEFAULT IT RUNS ON SYNTHETIC RECORDS.** Point it at a real canonical
//! stream to run it on real history, which is the only kind worth quoting:
//!
//! ```sh
//! tools/canon.py <aaps.db> -o records.ndjson
//! DIASWARM_RECORDS=records.ndjson \
//!   cargo test --release --test readcost_compare -- --nocapture
//! ```
//!
//! Release, not debug: curve25519 in a debug build is not the curve25519 that
//! runs on the phone, and the ratio is the point.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use diaswarm_core::vault::{Identity, Vault as CoreVault};
use diaswarm_core::{EPOCH_MS, Record, epoch_of, kind};
use diaswarm_keys::{Vault as KeysVault, wire};
use p2panda_core::SigningKey;
use p2panda_encryption::Rng;
use p2panda_store::{SqliteStore, SqliteStoreBuilder};

const OFFSET: i64 = 12 * 3_600_000;

/// How many times each read is timed. The best is reported: the slow ones are
/// this machine doing something else, not the vault costing more.
const TRIALS: usize = 5;

fn tmp(tag: &str) -> PathBuf {
    let p = std::env::temp_dir().join(format!(
        "diaswarm-readcmp-{tag}-{}",
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
            .filter(|r| r.kind() != kind::META)
            .collect();
        assert!(!parsed.is_empty(), "{path} parsed to no records");
        return (parsed, "real");
    }

    let mut out = Vec::new();
    for day in 0..50i64 {
        let base = (22_000 + day) * EPOCH_MS;
        for slot in 0..288i64 {
            out.push(
                Record::new(base + slot * 300_000, "cgm")
                    .set("mgdl", Some((90.0 + (slot as f64 * 3.1) % 140.0).into())),
            );
        }
    }
    (out, "synthetic")
}

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

fn best(mut f: impl FnMut() -> usize) -> (Duration, usize) {
    let mut fastest = Duration::MAX;
    let mut n = 0;
    for _ in 0..TRIALS {
        let t = Instant::now();
        n = f();
        fastest = fastest.min(t.elapsed());
    }
    (fastest, n)
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

#[tokio::test(flavor = "multi_thread")]
async fn read_cost_of_both_vaults_against_history_held() {
    let (all, source) = records();
    let mut by_epoch: BTreeMap<i64, Vec<Record>> = BTreeMap::new();
    for r in &all {
        by_epoch.entry(epoch_of(r.t(), OFFSET)).or_default().push(r.clone());
    }
    let epochs: Vec<i64> = by_epoch.keys().copied().collect();
    eprintln!(
        "\n  {source}: {} records over {} epochs, best of {TRIALS} reads\n",
        all.len(),
        epochs.len()
    );

    let sizes: Vec<usize> = [7usize, 14, 28, epochs.len()]
        .into_iter()
        .filter(|n| *n <= epochs.len())
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect();

    eprintln!(
        "  {:>6}  {:>8} | {:>12}  {:>12} | {:>12}  {:>12}",
        "epochs", "records", "core whole", "core recent", "keys whole", "keys recent"
    );
    eprintln!("  {}", "-".repeat(84));

    for n in sizes {
        // The OLDEST n epochs, so "recent" is always the newest of the set and
        // the set grows by adding history behind it — which is what a vault
        // that has been running for n days actually looks like.
        let window: Vec<i64> = epochs.iter().copied().take(n).collect();
        let newest = *window.last().expect("non-empty window");
        let held: usize = window.iter().map(|e| by_epoch[e].len()).sum();

        // ---- the shipping vault ----
        let core_root = tmp("core");
        let subject = Identity::generate();
        let reader = Identity::generate();
        let core = CoreVault::create(&core_root, &subject, OFFSET).expect("core vault");
        for e in &window {
            core.seal(*e, &by_epoch[e]).expect("core seal");
        }
        let reader_pub: [u8; 32] = x25519_dalek::PublicKey::from(&reader.encryption).to_bytes();
        core.record_grant(&subject, &reader_pub, "follow", "grant", 0).expect("core grant");
        core.publish_wraps(&subject, &reader_pub, "follow").expect("core wraps");

        let (core_whole, core_n) = best(|| {
            core.read_as(&reader, "follow").expect("core read all").values().map(Vec::len).sum()
        });
        let (core_recent, _) = best(|| {
            core.read_as_from(&reader, "follow", newest)
                .expect("core read recent")
                .values()
                .map(Vec::len)
                .sum()
        });

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
        for e in &window {
            keys.seal(*e, &by_epoch[e]).expect("keys seal");
        }
        let mut keys_reader = KeysVault::open(&keys_root, OFFSET, &reader_key).expect("reader");
        let registry =
            KeysVault::registry(&[(keys.subject(), subject_bundle.clone())]).expect("registry");
        let store = SqliteStoreBuilder::memory().build().await.expect("store");
        let welcome = deliver(&store, &subject_key, &welcome).await;
        keys_reader.join(reader_mgr, registry, &subject_bundle, "follow", &welcome).expect("join");

        let (keys_whole, keys_n) = best(|| {
            keys_reader.read_from(i64::MIN).expect("keys read all").values().map(Vec::len).sum()
        });
        let (keys_recent, _) = best(|| {
            keys_reader.read_from(newest).expect("keys read recent").values().map(Vec::len).sum()
        });

        eprintln!(
            "  {:>6}  {:>8} | {:>10.1}ms  {:>10.1}ms | {:>10.1}ms  {:>10.1}ms",
            n,
            held,
            ms(core_whole),
            ms(core_recent),
            ms(keys_whole),
            ms(keys_recent),
        );

        // A timing table nobody can trust is worse than none: if the two
        // vaults did not return the same number of records, the times are
        // measuring different work and the comparison is void.
        assert_eq!(
            core_n, keys_n,
            "at {n} epochs the vaults returned different record counts \
             ({core_n} vs {keys_n}) — the timings above are not comparable"
        );
    }

    eprintln!(
        "\n  'recent' is one epoch. If a recent-read column is flat, a follower \
         watching\n  today pays the same whatever the subject holds; if it rises, \
         it does not.\n"
    );
}
