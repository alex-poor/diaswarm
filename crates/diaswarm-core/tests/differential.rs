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

/// The reference snapshot's own standing offset, which canon.py derives from it.
const OFFSET: i64 = 12 * 3_600_000;

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
    // Read the offset back rather than assuming it: canon.py derives the phase
    // from the data's own utcOffset, and hard-coding it here would test this
    // test's guess instead of the two implementations agreeing.
    let parsed = Record::from_json(first).expect("the header is a record");
    let offset = parsed.get("offset").and_then(|v| v.as_i64()).expect("a declared offset");
    assert_eq!(header(offset).to_canonical_json(), first);
    assert_eq!(offset, OFFSET, "the fixture should carry a +12h utcOffset");
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
fn epochs_are_days_cut_at_a_fixed_offset() {
    assert_eq!(epoch_of(0, 0), 0);
    assert_eq!(epoch_of(EPOCH_MS - 1, 0), 0);
    assert_eq!(epoch_of(EPOCH_MS, 0), 1);
    // Negative timestamps are before 1970 and should floor, not truncate
    // toward zero — otherwise two epochs share an index either side of it.
    assert_eq!(epoch_of(-1, 0), -1);
    // The offset moves the phase, not the width. At +12h the boundary lands at
    // local midnight rather than local noon, which is the whole point of it.
    assert_eq!(epoch_of(0, OFFSET), 0);
    assert_eq!(epoch_of(EPOCH_MS / 2 - 1, OFFSET), 0);
    assert_eq!(epoch_of(EPOCH_MS / 2, OFFSET), 1);
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
    let out = encode(&records, OFFSET);
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

#[test]
fn normalising_a_canonical_stream_moves_nothing() {
    // Idempotence is the whole claim: canon.py already applied these rules, so
    // if the Rust table disagrees anywhere it shows up as a changed line.
    let Ok(path) = std::env::var("DIASWARM_STREAM") else { return };
    let stream = std::fs::read_to_string(&path).expect("stream is readable");
    let mut checked = 0usize;
    for (n, line) in stream.lines().enumerate() {
        let record = Record::from_json(line).expect("valid JSON");
        assert_eq!(
            record.normalise().to_canonical_json(),
            line,
            "line {} changed under normalisation — the precision table disagrees \
             with tools/canon.py",
            n + 1
        );
        checked += 1;
    }
    eprintln!("  {checked} records unchanged by normalisation");
}

#[test]
fn rounding_matches_python_ties_to_even() {
    // Python: round(0.125, 2) == 0.12, round(0.135, 2) == 0.14
    let cgm = |v: f64| {
        Record::new(1, "cgm")
            .set("mgdl", Some(v.into()))
            .normalise()
            .get("mgdl")
            .and_then(|x| x.as_f64())
            .unwrap()
    };
    assert_eq!(cgm(100.25), 100.2, "ties round to even, as Python does");
    assert_eq!(cgm(100.35), 100.4);
    assert_eq!(cgm(163.04), 163.0);
}

#[test]
fn a_mmol_profile_is_normalised_to_mgdl_and_loses_its_unit() {
    // §2: cgm.mgdl and target.lo are mg/dL always, profile blocks are in the
    // user's own unit. Untouched, one stream carries a target of 5 and a target
    // of 160.2 meaning nearly the same thing.
    let r = Record::new(1, "profile")
        .set("name", Some("BAU".into()))
        .set("unit", Some("MMOL".into()))
        .set("isf", Some(serde_json::json!([{"duration": 86400000, "amount": 2.0}])))
        .set(
            "target",
            Some(serde_json::json!([{"duration": 86400000, "lowTarget": 5.0, "highTarget": 6.0}])),
        )
        .set("basal", Some(serde_json::json!([{"duration": 86400000, "amount": 0.5}])))
        .normalise();

    assert!(r.get("unit").is_none(), "a normalised profile still declares a unit");
    assert_eq!(r.get("isf").unwrap()[0]["amount"].as_f64(), Some(36.0));
    assert_eq!(r.get("target").unwrap()[0]["lowTarget"].as_f64(), Some(90.1));
    assert_eq!(r.get("target").unwrap()[0]["highTarget"].as_f64(), Some(108.1));
    // U/h and g/U carry no glucose unit and must never be scaled.
    assert_eq!(r.get("basal").unwrap()[0]["amount"].as_f64(), Some(0.5));
}

#[test]
fn an_mgdl_profile_is_left_alone_but_still_loses_its_unit() {
    let r = Record::new(1, "profile")
        .set("unit", Some("MGDL".into()))
        .set("isf", Some(serde_json::json!([{"duration": 1, "amount": 36.0}])))
        .normalise();
    assert!(r.get("unit").is_none());
    assert_eq!(r.get("isf").unwrap()[0]["amount"].as_f64(), Some(36.0));
}

#[test]
fn an_unrecognised_unit_is_admitted_rather_than_guessed() {
    // A visible "not normalised" beats a plausible wrong number.
    let r = Record::new(1, "profile")
        .set("unit", Some("FURLONGS".into()))
        .set("isf", Some(serde_json::json!([{"duration": 1, "amount": 2.0}])))
        .normalise();
    assert_eq!(r.get("unit").and_then(|v| v.as_str()), Some("FURLONGS"));
    assert_eq!(r.get("isf").unwrap()[0]["amount"].as_f64(), Some(2.0), "scaled a unit it did not know");
}
