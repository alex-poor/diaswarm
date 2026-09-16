//! What does the follower's tail fetch cost, and what would each fix save?
//!
//! **WHERE THE FOLLOWER'S CPU ACTUALLY GOES.** A symbolised profile of Ayni on
//! 2026-09-17 (phone B, 30s, `-g`) put 64% of the process's cycles in the
//! native library and 28% in the allocator. Resolving the curve25519 leaves by
//! caller:
//!
//! ```text
//! ~493 samples  ed25519_dalek verify <- p2panda_core::VerifyingKey::verify
//!                 <- AnyHeader::decode <- Header<wire::KeysArgs>::decode
//!  ~18 samples  x25519 diffie_hellman <- diaswarm_core::seal::unwrap
//!                 <- diaswarm_core::vault::Vault::read_as_from
//! ```
//!
//! So 96% of it is verifying operation-header signatures, and the vault read
//! path — which an earlier reading of this blamed — is about 3%. A separate
//! benchmark (`readcost_compare`) confirms the other half: recent-end vault
//! reads are flat at 0.7 ms whatever history is held, in BOTH vaults.
//!
//! The mechanism is [`crate::follow`]'s tail: it asks for `tail_days * 320`
//! operations capped at 20,000, [`wire::segments_tail`] fetches every one of
//! them, and `AnyHeader::decode` runs an Ed25519 verification per header
//! before most are discarded as supersets. follow.rs predicts this in its own
//! comment — "the thing to revisit first if replication gets expensive".
//!
//! The log grows by one operation per FLUSH, not per day: the plugin drains on
//! a cadence and a Libre 3 reports every minute, so 20,000 operations is about
//! a fortnight and the cap is reached rather than approached.
//!
//! **THE THREE CANDIDATE FIXES ALL REDUCE TO DECODING FEWER OPERATIONS**, so
//! this prices one log at several fetch widths and reads each fix off that
//! curve, rather than half-implementing three things and trusting three
//! numbers:
//!
//! | fix                         | operations it would decode          |
//! |-----------------------------|-------------------------------------|
//! | 2. fetch by epoch           | those in the epochs actually wanted  |
//! | 3. cache decoded by seq_num | those added since the last read      |
//! | 1. skip re-verification     | all of them, minus the Ed25519 part  |
//!
//! Fix 1 is priced separately, because `AnyHeader::decode` verifies
//! unconditionally and there is no unverified decode in p2panda's public API —
//! so that one needs an upstream change or a vendored patch, and it is worth
//! knowing what it would buy before asking for either.
//!
//! ```sh
//! tools/canon.py <aaps.db> -o records.ndjson
//! DIASWARM_RECORDS=records.ndjson \
//!   cargo test --release --test tailcost -- --nocapture
//! ```

use std::path::PathBuf;
use std::time::{Duration, Instant};

use diaswarm_core::{EPOCH_MS, Record, epoch_of, kind};
use diaswarm_keys::{SqliteStoreBuilder, Vault as KeysVault, wire};
use p2panda_core::SigningKey;
use p2panda_encryption::Rng;
use p2panda_store::SqliteStore;

const OFFSET: i64 = 12 * 3_600_000;

/// One flush of the AAPS plugin: what it has collected since the last drain.
/// A Libre 3 reports every minute, so this is about a minute of data.
const PER_FLUSH: usize = 2;

/// The log the phone actually has after a fortnight, and the cap follow.rs hits.
const LOG_OPS: usize = 20_000;

const TRIALS: usize = 3;

fn tmp(tag: &str) -> PathBuf {
    let p = std::env::temp_dir().join(format!(
        "diaswarm-tailcost-{tag}-{}",
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
    for day in 0..14i64 {
        let base = (22_000 + day) * EPOCH_MS;
        for slot in 0..1440i64 {
            out.push(
                Record::new(base + slot * 60_000, "cgm")
                    .set("mgdl", Some((90.0 + (slot as f64 * 3.1) % 140.0).into())),
            );
        }
    }
    (out, "synthetic")
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

async fn time_tail(store: &SqliteStore, author: &p2panda_core::VerifyingKey, k: u64) -> Duration {
    let mut fastest = Duration::MAX;
    for _ in 0..TRIALS {
        let t = Instant::now();
        let got = wire::segments_tail(store, author, k).await.expect("tail");
        fastest = fastest.min(t.elapsed());
        std::hint::black_box(got.len());
    }
    fastest
}

#[tokio::test(flavor = "multi_thread")]
async fn what_the_tail_fetch_costs_and_what_each_fix_would_save() {
    let (all, source) = records();

    // ---- build ONE realistic log, then price reads against it ----
    let root = tmp("keys");
    let subject_key = SigningKey::from_bytes(&rand32());
    let rng = Rng::default();
    let mut keys = KeysVault::open(&root, OFFSET, &subject_key).expect("vault");
    let (subject_mgr, _bundle) = KeysVault::key_bundle(&rng).expect("bundle");
    keys.create(subject_mgr).expect("create");
    let store = SqliteStoreBuilder::memory().build().await.expect("store");
    let author = subject_key.verifying_key();

    let flushes: Vec<&[Record]> = all.chunks(PER_FLUSH).take(LOG_OPS).collect();
    let ops = flushes.len();
    let build = Instant::now();
    let mut per_epoch_last = 0u64;
    let last_epoch = epoch_of(all.last().expect("records").t(), OFFSET);
    for chunk in &flushes {
        let epoch = epoch_of(chunk[0].t(), OFFSET);
        if epoch == last_epoch {
            per_epoch_last += 1;
        }
        let segment = keys.seal_delta(epoch, chunk).expect("seal_delta");
        wire::publish(&store, &subject_key, &segment).await.expect("publish");
    }
    let epochs_spanned = {
        let first = epoch_of(all[0].t(), OFFSET);
        (last_epoch - first + 1).max(1)
    };
    eprintln!(
        "\n  {source}: {} records -> {ops} operations ({PER_FLUSH}/flush) over {epochs_spanned} epochs, built in {:.1}s",
        all.len(),
        build.elapsed().as_secs_f64()
    );
    eprintln!("  operations in the newest epoch: {per_epoch_last}\n");

    // ---- the curve: cost against operations decoded ----
    eprintln!("  {:>10}  {:>12}  {:>14}", "ops fetched", "time", "per op");
    eprintln!("  {}", "-".repeat(40));
    let mut baseline = Duration::ZERO;
    let mut widths: Vec<u64> = vec![1, 60, per_epoch_last.max(1), 5_000, ops as u64];
    widths.sort_unstable();
    widths.dedup();
    for k in widths {
        let d = time_tail(&store, &author, k).await;
        if k == ops as u64 {
            baseline = d;
        }
        eprintln!("  {:>10}  {:>10.1}ms  {:>12.1}µs", k, ms(d), d.as_secs_f64() * 1e6 / k as f64);
    }

    // ---- fix 1, priced on its own ----
    //
    // `AnyHeader::decode` verifies unconditionally, so this is what a fix would
    // have to remove, measured directly rather than inferred from a subtraction.
    use ed25519_dalek::{Signer, SigningKey as EdKey, Verifier};
    let ed = EdKey::from_bytes(&rand32());
    // A header's signed bytes are the CBOR tuple minus the signature: small.
    let msg = vec![0xA5u8; 160];
    let sig = ed.sign(&msg);
    let vk = ed.verifying_key();
    let t = Instant::now();
    for _ in 0..ops {
        std::hint::black_box(vk.verify(&msg, &sig).is_ok());
    }
    let verify_only = t.elapsed();

    // ---- fix 2, as shipped: walk back from the tip instead of guessing ----
    //
    // The same question a follower asks — "the newest day" — put to the old
    // fixed-width tail and to `segments_covering`.
    let mut covering = Duration::MAX;
    for _ in 0..TRIALS {
        let t = Instant::now();
        let got = wire::segments_covering(&store, &author, last_epoch, 1, 320)
            .await
            .expect("covering");
        covering = covering.min(t.elapsed());
        std::hint::black_box(got.len());
    }
    let covered = wire::segments_covering(&store, &author, last_epoch, 1, 320)
        .await
        .expect("covering");
    // ---- fix 3: the other three readers in the same refresh ----
    //
    // Cold, then the repeat the other `keys*` accessors make against an
    // unchanged log. Measured in that order because the cache is keyed on the
    // tip and a warm entry is exactly what the second reader finds.
    wire::forget_cached_segments();
    let t = Instant::now();
    let cold = wire::segments_covering(&store, &author, last_epoch, 1, 320).await.expect("cold");
    let cold_t = t.elapsed();
    let mut warm_t = Duration::MAX;
    for _ in 0..TRIALS {
        let t = Instant::now();
        let warm = wire::segments_covering(&store, &author, last_epoch, 1, 320).await.expect("warm");
        warm_t = warm_t.min(t.elapsed());
        // A cache that returns less than the read it stands in for is a bug
        // that would show up as missing glucose on a screen, not as a panic.
        assert_eq!(
            warm.len(),
            cold.len(),
            "warm read returned {} segments where the cold read returned {}",
            warm.len(),
            cold.len()
        );
    }

    let old_width = (ops as u64).min(320);
    let old_tail = wire::segments_tail(&store, &author, (1u64 * 320).min(20_000))
        .await
        .expect("tail");
    // The fix must not return less of the wanted epoch than the old path did.
    let want_new = covered.iter().filter(|s| s.epoch == last_epoch).count();
    let want_old = old_tail.iter().filter(|s| s.epoch == last_epoch).count();
    assert!(
        want_new >= want_old,
        "segments_covering returned {want_new} segments for the newest epoch \
         where the old tail returned {want_old} — the fix is dropping data"
    );
    let _ = old_width;

    eprintln!("\n  {}", "=".repeat(64));
    eprintln!("  baseline  fetch {ops} ops (what follow.rs did)         {:>10.1}ms", ms(baseline));
    eprintln!(
        "  FIX 2     segments_covering, cold                    {:>10.1}ms  ({} segments, {:.0}x faster)",
        ms(cold_t),
        covered.len(),
        baseline.as_secs_f64() / cold_t.as_secs_f64()
    );
    eprintln!(
        "  FIX 3     the same window, warm (3 more readers)     {:>10.3}ms  ({:.0}x faster)",
        ms(warm_t),
        baseline.as_secs_f64() / warm_t.as_secs_f64()
    );
    let refresh_before = baseline.as_secs_f64() * 4.0;
    let refresh_after = cold_t.as_secs_f64() + warm_t.as_secs_f64() * 3.0;
    eprintln!(
        "\n  one refresh (4 readers)   before {:>8.1}ms   after {:>7.1}ms   {:.0}x",
        refresh_before * 1000.0,
        refresh_after * 1000.0,
        refresh_before / refresh_after
    );
    let _ = covering;
    eprintln!(
        "  of which  {ops} Ed25519 verifications                   {:>10.1}ms  ({:.0}%)",
        ms(verify_only),
        100.0 * verify_only.as_secs_f64() / baseline.as_secs_f64()
    );
    eprintln!("  {}", "=".repeat(64));
    eprintln!(
        "\n  fix 1  skip re-verification   saves the {:.0}% above, needs an upstream\n         \
         or vendored p2panda change: decode() verifies unconditionally.",
        100.0 * verify_only.as_secs_f64() / baseline.as_secs_f64()
    );
    eprintln!(
        "  fix 2  fetch by epoch         decodes {per_epoch_last} not {ops} for the newest epoch."
    );
    eprintln!("  fix 3  cache by seq_num       decodes the delta since the last read.\n");

    assert!(ops > 0, "no operations were published");
}
