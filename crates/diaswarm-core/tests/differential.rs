//! Does this agree with `tools/canon.py`, byte for byte?
//!
//! The reason to put the record logic in one Rust crate over the NDK was that
//! two implementations of a frozen contract drift. That claim is only worth
//! anything if it is checked, so these tests run the Python emitter over the
//! synthetic fixture and assert that re-encoding its output in Rust reproduces
//! it exactly — key order, float formatting, absent fields and all.
//!
//! They need `python3` and the repo's own tools, so they are dev-time checks
//! rather than something the Android build runs.

use std::path::{Path, PathBuf};
use std::process::Command;

use diaswarm_core::{debounce_cgm, encode, epoch_of, header, sort, Record, EPOCH_MS};

fn repo_root() -> PathBuf {
    // crates/diaswarm-core -> repo root
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("repo root")
        .to_path_buf()
}

/// Build the fixture and canonicalise it with the Python emitter.
fn python_stream() -> Option<String> {
    let root = repo_root();
    let tmp = std::env::temp_dir().join("diaswarm-differential.db");
    let _ = std::fs::remove_file(&tmp);

    let made = Command::new("python3")
        .arg(root.join("tools/mkfixture.py"))
        .arg(&tmp)
        .output()
        .ok()?;
    if !made.status.success() {
        return None;
    }

    let out = Command::new("python3")
        .arg(root.join("tools/canon.py"))
        .arg(&tmp)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    String::from_utf8(out.stdout).ok()
}

#[test]
fn canonical_encoding_is_byte_identical_to_the_python_emitter() {
    let Some(stream) = python_stream() else {
        eprintln!("skipping: python3 or the repo tools are unavailable");
        return;
    };
    assert!(!stream.is_empty(), "the Python emitter produced nothing");

    let mut lines = 0;
    for line in stream.lines() {
        let record = Record::from_json(line).expect("canon.py emits valid JSON");
        assert_eq!(
            record.to_canonical_json(),
            line,
            "re-encoding differs from the Python emitter — the two implementations \
             have drifted, which is the exact failure putting this in one crate was \
             meant to prevent"
        );
        lines += 1;
    }
    assert!(lines > 10, "expected a stream, got {lines} lines");
}

#[test]
fn the_header_matches_the_python_header() {
    let Some(stream) = python_stream() else { return };
    let first = stream.lines().next().expect("a header line");
    assert_eq!(header().to_canonical_json(), first);
}

#[test]
fn ordering_reproduces_the_python_order() {
    let Some(stream) = python_stream() else { return };
    let original: Vec<Record> = stream
        .lines()
        .skip(1) // the header is not an event
        .map(|l| Record::from_json(l).unwrap())
        .collect();

    // Shuffle deterministically, then re-sort: the canonical order must be a
    // property of the records, not of the order they arrived in.
    let mut shuffled = original.clone();
    shuffled.reverse();
    sort(&mut shuffled);

    assert_eq!(
        shuffled.iter().map(|r| r.to_canonical_json()).collect::<Vec<_>>(),
        original.iter().map(|r| r.to_canonical_json()).collect::<Vec<_>>(),
        "sorting did not reproduce the emitted order"
    );
}

#[test]
fn absence_survives_the_round_trip() {
    let Some(stream) = python_stream() else { return };
    // The fixture carries a carb entry with no duration beside one with a real
    // zero. If either turns into the other, §1's distinction is gone.
    let carbs: Vec<Record> = stream
        .lines()
        .filter_map(|l| Record::from_json(l).ok())
        .filter(|r| r.kind() == "carb")
        .collect();
    assert!(carbs.len() >= 2, "fixture should carry two carb entries");

    let no_dur = carbs.iter().filter(|r| r.get("dur").is_none()).count();
    let zero_dur = carbs
        .iter()
        .filter(|r| r.get("dur").and_then(|v| v.as_i64()) == Some(0))
        .count();
    assert_eq!(no_dur, 1, "a carb with an unreported duration must have no `dur`");
    assert_eq!(zero_dur, 1, "a carb with a real zero must keep it");
}

#[test]
fn epochs_are_utc_days() {
    assert_eq!(epoch_of(0), 0);
    assert_eq!(epoch_of(EPOCH_MS - 1), 0);
    assert_eq!(epoch_of(EPOCH_MS), 1);
    // Negative timestamps are before 1970 and should floor, not truncate
    // toward zero — otherwise two epochs share an index either side of it.
    assert_eq!(epoch_of(-1), -1);
}

#[test]
fn debounce_keeps_the_first_reading_of_a_bucket() {
    let bucket = 1_699_999_800_000;
    let first = Record::new(bucket, "cgm").set("mgdl", Some(100.0.into()));
    let second = Record::new(bucket + 120_000, "cgm").set("mgdl", Some(150.0.into()));
    let (kept, dropped) = debounce_cgm(vec![first, second]);
    assert_eq!(dropped, 1);
    assert_eq!(kept.len(), 1);
    assert_eq!(kept[0].get("mgdl").unwrap().as_f64(), Some(100.0));
}

#[test]
fn encode_emits_a_header_then_one_line_per_record() {
    let records = vec![Record::new(1, "cgm").set("mgdl", Some(100.0.into()))];
    let out = encode(&records);
    assert_eq!(out.lines().count(), 2);
    assert!(out.ends_with('\n'));
}

/// The fixture is small and its numbers are round. Real history is neither, and
/// float formatting is where two JSON implementations actually diverge. Point
/// this at a real canonicalised stream:
///
///     DIASWARM_STREAM=/path/to/stream.ndjson cargo test -- --nocapture
#[test]
fn byte_identical_on_a_real_stream() {
    let Ok(path) = std::env::var("DIASWARM_STREAM") else {
        eprintln!("skipping: set DIASWARM_STREAM to a canonicalised stream");
        return;
    };
    let stream = std::fs::read_to_string(&path).expect("stream is readable");
    let mut checked = 0usize;
    for (n, line) in stream.lines().enumerate() {
        let record = Record::from_json(line).expect("valid JSON");
        assert_eq!(
            record.to_canonical_json(),
            line,
            "line {} differs between implementations",
            n + 1
        );
        checked += 1;
    }
    eprintln!("  {checked} records re-encoded byte-identically");
    assert!(checked > 1000, "expected a real stream, got {checked} records");
}
