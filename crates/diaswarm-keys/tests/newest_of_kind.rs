//! `follow::newest_of_kind` finds the single newest record of one kind by folding
//! over segments, never collecting the history.
//!
//! **THE CATCH-UP OOM THIS REPLACES (issue #3).** A temp target or profile in
//! force can be weeks old, so their readers ask from `i64::MIN` — the whole log.
//! `follow::read` answers that by building a `Vec` of every record plus a `seen`
//! set plus an ndjson `String`, the ~22× blow-up that SIGABRT-crashed a follower
//! on launch. The fold keeps one record. This pins the behaviour the readers
//! depend on: the newest by `t`, across epochs, bounded by `from_epoch`, and
//! `None` when the kind was never sealed.

use diaswarm_core::{EPOCH_MS, Record};
use diaswarm_keys::{follow, wire, Rng, SqliteStoreBuilder, Vault};
use p2panda_core::SigningKey;

const OFFSET: i64 = 12 * 3_600_000;

fn tmp(tag: &str) -> std::path::PathBuf {
    let p = std::env::temp_dir().join(format!(
        "diaswarm-newest-{tag}-{}",
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
    ));
    std::fs::create_dir_all(&p).unwrap();
    p
}

fn target(epoch: i64, slot: i64, hi: f64) -> Record {
    Record::new(epoch * EPOCH_MS + slot * 60_000, "target")
        .set("lo", Some(90.0.into()))
        .set("hi", Some(hi.into()))
}

#[tokio::test(flavor = "multi_thread")]
async fn newest_of_kind_folds_to_the_latest_by_t_across_epochs() {
    let key = SigningKey::from_bytes(&[11u8; 32]);
    let author = key.verifying_key();
    let rng = Rng::default();
    let mut vault = Vault::open(tmp("subject"), OFFSET, &key).unwrap();
    let (mgr, _bundle) = Vault::key_bundle(&rng).unwrap();
    vault.create(mgr).unwrap();
    let store = SqliteStoreBuilder::memory().build().await.unwrap();

    // An OLD target on day 40_000, a cgm reading between, and a NEWER target two
    // days later. Each in its own segment, as a per-flush publisher produces.
    let older = target(40_000, 1, 120.0);
    let cgm = Record::new(40_001 * EPOCH_MS, "cgm").set("mgdl", Some(105.0.into()));
    let newer = target(40_002, 1, 110.0); // the one in force — identified by hi=110

    for (epoch, rec) in [(40_000, &older), (40_001, &cgm), (40_002, &newer)] {
        let seg = vault.seal_delta(epoch, std::slice::from_ref(rec)).unwrap();
        wire::publish(&store, &key, &seg).await.unwrap();
    }

    // The whole log: the newest target wins, and only one record comes back.
    let (best, opened, skipped) =
        follow::newest_of_kind(&vault, &store, &author, "target", i64::MIN).await.unwrap();
    assert_eq!(opened, 3, "all three segments were opened");
    assert_eq!(skipped.lost(), 0, "the subject opens its own segments");
    let best = best.expect("a target is in force");
    assert_eq!(best.kind(), "target");
    assert_eq!(best.t(), newer.t(), "the newest target by t, not the first found");
    assert_eq!(best.get("hi").and_then(|v| v.as_f64()), Some(110.0), "the in-force target");

    // Any kind, same fold: the one cgm reading.
    let (cgm_best, _, _) =
        follow::newest_of_kind(&vault, &store, &author, "cgm", i64::MIN).await.unwrap();
    assert_eq!(cgm_best.map(|r| r.t()), Some(cgm.t()));

    // A kind that was never sealed is None, not an error.
    let (none, _, _) =
        follow::newest_of_kind(&vault, &store, &author, "profile", i64::MIN).await.unwrap();
    assert!(none.is_none(), "no profile was ever sealed");

    // `from_epoch` bounds by segment epoch: past the newest target's day, nothing.
    let (bounded, _, _) =
        follow::newest_of_kind(&vault, &store, &author, "target", 40_003).await.unwrap();
    assert!(bounded.is_none(), "from_epoch after both targets excludes them");

    // And a floor between the two targets drops the older, keeps the newer.
    let (between, _, _) =
        follow::newest_of_kind(&vault, &store, &author, "target", 40_001).await.unwrap();
    assert_eq!(
        between.and_then(|r| r.get("hi").and_then(|v| v.as_f64())),
        Some(110.0),
        "from_epoch above the old target still finds the newer one",
    );
}
