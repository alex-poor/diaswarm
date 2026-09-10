//! Do both vaults give a granted reader the same records, over real history?
//!
//! THE TEST THAT WOULD JUSTIFY DELETING `vault.rs`, and the reason this crate
//! was built alongside `diaswarm-core` instead of replacing it. Everything else
//! measured is a property — a stranger cannot read, a revocation cuts, a grant
//! reaches back. This asks the duller and more important question: put a real
//! person's diabetes history through both implementations and does the same
//! data come out?
//!
//! A migration that quietly drops a bolus is worse than no migration, and the
//! hand-rolled vault has dropped records before — 4,276 of them, from the only
//! copy a subject had, because a seal wrote over a segment instead of appending.
//! That is what this exists to catch.
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
//!
//! `DIASWARM_EPOCHS=n` caps how many days are used, which is how the per-
//! operation timings below were taken without waiting for a full history.

use std::collections::BTreeMap;
use std::path::PathBuf;

use diaswarm_core::vault::{Identity, Vault as CoreVault};
use diaswarm_core::{EPOCH_MS, Record, epoch_of, kind};
use diaswarm_spaces::{Reach, Vault};

const OFFSET: i64 = 12 * 3_600_000;

fn tmp(tag: &str) -> PathBuf {
    let p = std::env::temp_dir().join(format!(
        "diaswarm-diff-{tag}-{}",
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
    ));
    std::fs::create_dir_all(&p).unwrap();
    p
}

/// Real history if it was offered, synthetic otherwise.
fn records() -> (Vec<Record>, &'static str) {
    if let Ok(path) = std::env::var("DIASWARM_RECORDS") {
        let text = std::fs::read_to_string(&path).expect("DIASWARM_RECORDS is not readable");
        let parsed: Vec<Record> = text
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .filter_map(|l| Record::from_json(l).ok())
            // The stream header is emitted by each implementation for itself,
            // once per segment in one and once per window in the other, so
            // counting them would compare bookkeeping rather than data.
            .filter(|r| r.kind() != kind::META)
            .collect();
        assert!(!parsed.is_empty(), "{path} parsed to no records");
        return (parsed, "real");
    }

    // Enough shape to be worth comparing: several days, several kinds, and
    // readings close enough together to exercise the canonical ordering.
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

fn by_epoch(records: &[Record]) -> BTreeMap<i64, Vec<Record>> {
    let mut out: BTreeMap<i64, Vec<Record>> = BTreeMap::new();
    for r in records {
        out.entry(epoch_of(r.t(), OFFSET)).or_default().push(r.clone());
    }
    out
}

fn canonical(records: &[Record]) -> Vec<String> {
    let mut out: Vec<String> = records
        .iter()
        .filter(|r| r.kind() != kind::META)
        .map(|r| r.to_canonical_json())
        .collect();
    out.sort();
    out
}

#[tokio::test]
async fn both_vaults_give_a_reader_the_same_history() {
    let (source, provenance) = records();
    let mut days = by_epoch(&source);
    if let Ok(n) = std::env::var("DIASWARM_EPOCHS") {
        let n: usize = n.parse().expect("DIASWARM_EPOCHS must be a number");
        days = days.into_iter().take(n).collect();
    }
    println!(
        "  {provenance} history: {} records over {} epochs",
        source.len(),
        days.len()
    );

    // --- the hand-rolled vault -------------------------------------------
    let core_reader = Identity::generate();
    let core_subject = Identity::generate();
    let core_dir = tmp("core");
    let core = CoreVault::create(&core_dir, &core_subject, OFFSET).unwrap();
    core.record_grant(&core_subject, &core_reader.enc_public(), "follow", "grant", 0)
        .unwrap();
    for (epoch, day) in &days {
        core.seal(*epoch, day).unwrap();
    }
    let core_read: Vec<Record> = core
        .read_as(&core_reader, "follow")
        .unwrap()
        .into_values()
        .flatten()
        .collect();

    // --- the same history, on p2panda-spaces ------------------------------
    let mut subject = Vault::open(tmp("spaces-subject"), OFFSET).await.unwrap();
    let reader = Vault::open(tmp("spaces-reader"), OFFSET).await.unwrap();
    subject.register(&reader).await.unwrap();
    reader.register(&subject).await.unwrap();

    let mut ops = subject.seal(&[]).await.unwrap();
    ops.extend(subject.grant(reader.subject(), Reach::Everything).await.unwrap());

    // TIMED, because the answer decides whether this can run on a phone at all.
    let t0 = std::time::Instant::now();
    for day in days.values() {
        ops.extend(subject.seal(day).await.unwrap());
    }
    let sealing = t0.elapsed();

    let t1 = std::time::Instant::now();
    let ingested = reader.ingest(&ops).await.unwrap();
    let reading = t1.elapsed();

    println!(
        "  {} operations · sealing {:.1}s ({:.0}ms/op) · reading {:.1}s ({:.0}ms/op)",
        ops.len(),
        sealing.as_secs_f64(),
        sealing.as_secs_f64() * 1000.0 / ops.len() as f64,
        reading.as_secs_f64(),
        reading.as_secs_f64() * 1000.0 / ops.len() as f64,
    );
    assert_eq!(
        ingested.panicked, 0,
        "{} operation(s) panicked, so this reader's history has a hole in it",
        ingested.panicked
    );

    // --- and they must agree ----------------------------------------------
    //
    // Built from `days`, not from `source`: with DIASWARM_EPOCHS set they are
    // different sets, and comparing against the untruncated one reports a
    // difference between the harness and itself.
    let sealed: Vec<Record> = days.values().flatten().cloned().collect();
    let want = canonical(&sealed);
    let got_core = canonical(&core_read);
    let got_spaces = canonical(&ingested.records);

    println!(
        "  sealed {} · core read {} · spaces read {}",
        want.len(),
        got_core.len(),
        got_spaces.len()
    );

    assert_eq!(
        got_core, want,
        "the hand-rolled vault did not return what was sealed into it"
    );
    assert_eq!(
        got_spaces, want,
        "the spaces vault did not return what was sealed into it"
    );
    assert_eq!(
        got_spaces, got_core,
        "the two implementations disagree — this is the migration's stop sign"
    );
}
