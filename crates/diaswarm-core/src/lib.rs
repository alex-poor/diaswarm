//! `spec/records.md` v1, as the one implementation.
//!
//! WHY THIS IS RUST AND NOT KOTLIN. The record shape is frozen, and every later
//! stage encodes against it. Two implementations of a frozen contract drift —
//! not in the obvious places but in float formatting, key order and rounding
//! mode — and the drift is invisible until two peers disagree about a stream
//! neither can prove wrong. So the phone and the desktop run the same code,
//! over the NDK, and `tools/canon.py` becomes the oracle this is tested against
//! rather than a second source of truth.
//!
//! WHAT IS SHARED AND WHAT IS NOT. The phone receives AAPS `DataPair` objects
//! from a `DataSyncSelector`; a snapshot tool reads SQLite. Those are different
//! inputs and neither belongs here. What belongs here is everything downstream
//! of them: the record shape, the canonical encoding, deduplication, and the
//! epoch a record falls in.

pub mod seal;
pub mod vault;

use std::collections::BTreeMap;

use serde_json::{Map, Value};

/// The version of `spec/records.md` this implements. Declared in the header.
pub const SPEC_VERSION: u64 = 2;

/// One epoch, one content key. UTC so an epoch has the same identity on every
/// device — see spec §5.1 for why local midnight is not an option.
pub const EPOCH_MS: i64 = 24 * 60 * 60 * 1000;
pub const EPOCH_BASIS: &str = "utc-day";

/// A CGM sensor produces one reading per five minutes. A property of the
/// hardware, not a tuning knob.
pub const CGM_BUCKET_MS: i64 = 5 * 60 * 1000;

/// Record kinds. A closed vocabulary (spec §2); `tdd` was removed in v1 and
/// `amend` is reserved for a v2 that has not been designed.
pub mod kind {
    pub const CGM: &str = "cgm";
    pub const BOLUS: &str = "bolus";
    pub const CARB: &str = "carb";
    pub const TBR: &str = "tbr";
    pub const EXT_BOLUS: &str = "extbolus";
    pub const EVENT: &str = "event";
    pub const TARGET: &str = "target";
    pub const PROFILE: &str = "profile";
    pub const META: &str = "meta";

    pub const ALL: &[&str] = &[CGM, BOLUS, CARB, TBR, EXT_BOLUS, EVENT, TARGET, PROFILE];
}

/// One event, at one instant, as the device recorded it.
///
/// Held as a sorted map rather than a struct per kind, because §1 makes absence
/// meaningful: a field the device did not report must be *missing*, not null and
/// not zero. A struct with `Option` fields plus `skip_serializing_if` expresses
/// the same thing and invites someone to add `#[serde(default)]` later, which
/// would quietly turn every absence into a zero.
#[derive(Clone, Debug, PartialEq)]
pub struct Record(BTreeMap<String, Value>);

impl Record {
    pub fn new(t: i64, kind: &str) -> Self {
        let mut m = BTreeMap::new();
        m.insert("t".into(), Value::from(t));
        m.insert("k".into(), Value::from(kind));
        Record(m)
    }

    /// Set a field, or leave it absent if the device reported nothing.
    pub fn set(mut self, key: &str, value: Option<Value>) -> Self {
        if let Some(v) = value {
            self.0.insert(key.into(), v);
        }
        self
    }

    pub fn t(&self) -> i64 {
        self.0.get("t").and_then(Value::as_i64).unwrap_or(0)
    }

    pub fn kind(&self) -> &str {
        self.0.get("k").and_then(Value::as_str).unwrap_or("")
    }

    pub fn get(&self, key: &str) -> Option<&Value> {
        self.0.get(key)
    }

    /// Which epoch this record falls in. The unit of key custody.
    pub fn epoch(&self) -> i64 {
        epoch_of(self.t())
    }

    /// Apply the spec's precision rules. Idempotent, and asserted so on real
    /// data: normalising an already-canonical stream must not move a single byte.
    pub fn normalise(mut self) -> Self {
        let kind = self.kind().to_string();
        for (key, value) in self.0.iter_mut() {
            let Some(digits) = precision(&kind, key) else { continue };
            let Some(n) = value.as_f64() else { continue };
            if let Some(rounded) = serde_json::Number::from_f64(round_half_even(n, digits)) {
                *value = Value::Number(rounded);
            }
        }
        self
    }

    /// The one canonical serialisation: keys sorted, no spaces.
    ///
    /// Byte-identical to `json.dumps(r, separators=(",", ":"), sort_keys=True)`
    /// in `tools/canon.py`, which is asserted against real output in the tests —
    /// because "the same JSON" is exactly the kind of thing two implementations
    /// agree about until they meet a float.
    pub fn to_canonical_json(&self) -> String {
        let mut map = Map::new();
        for (k, v) in &self.0 {
            map.insert(k.clone(), v.clone());
        }
        serde_json::to_string(&Value::Object(map)).expect("record is serialisable")
    }

    pub fn from_json(line: &str) -> Result<Self, serde_json::Error> {
        let value: Value = serde_json::from_str(line)?;
        let mut m = BTreeMap::new();
        if let Value::Object(obj) = value {
            for (k, v) in obj {
                if !v.is_null() {
                    m.insert(k, v);
                }
            }
        }
        Ok(Record(m))
    }
}

/// How many decimal places each numeric field carries, by kind.
///
/// The precision the device actually has. A pump delivering in 0.01 U steps has
/// no business emitting seventeen significant figures, and the extra digits cost
/// real bytes at one record every four minutes for a decade.
///
/// THIS TABLE IS WHY ROUNDING IS HERE AND NOT IN KOTLIN. It is part of the
/// canonical form: two emitters that round differently produce different bytes
/// for the same reading, and the difference is invisible until two peers compare
/// streams. The plugin passes raw doubles across the boundary and lets this
/// decide, which is the same reason the encoding lives here.
fn precision(kind: &str, key: &str) -> Option<i32> {
    match (kind, key) {
        (kind::CGM, "mgdl") | (kind::EVENT, "mgdl") => Some(1),
        (kind::CARB, "g") => Some(1),
        (kind::BOLUS, "u") | (kind::EXT_BOLUS, "u") => Some(3),
        (kind::TBR, "rate") => Some(3),
        (kind::TARGET, "lo") | (kind::TARGET, "hi") => Some(1),
        _ => None,
    }
}

/// Round the way Python's `round()` does — ties to even.
///
/// `tools/canon.py` is the oracle these implementations are tested against, and
/// it rounds half-to-even. Rust's `f64::round` rounds half away from zero, so
/// 0.125 at two places would be 0.13 here and 0.12 there: one reading in a
/// thousand differing, which is exactly the kind of drift that is never noticed
/// and never explained.
fn round_half_even(value: f64, digits: i32) -> f64 {
    let factor = 10f64.powi(digits);
    (value * factor).round_ties_even() / factor
}

pub fn epoch_of(t: i64) -> i64 {
    t.div_euclid(EPOCH_MS)
}

/// The stream header (spec §5.2). The only record that is not an event.
pub fn header() -> Record {
    Record::new(0, kind::META)
        .set("spec", Some(Value::from(SPEC_VERSION)))
        .set("epoch", Some(Value::from(EPOCH_BASIS)))
        .set("unit", Some(Value::from("mgdl")))
}

/// Sort into the canonical order: `(t, k)`, ties broken on the canonical
/// encoding so two implementations reading two snapshots agree (spec §1).
pub fn sort(records: &mut [Record]) {
    records.sort_by(|a, b| {
        (a.t(), a.kind())
            .cmp(&(b.t(), b.kind()))
            .then_with(|| a.to_canonical_json().cmp(&b.to_canonical_json()))
    });
}

/// One CGM reading per five-minute bucket, KEEPING THE FIRST.
///
/// The first is the reading the loop actually saw and acted on; a later
/// duplicate of the same measurement arrived after the decision was made.
/// Averaging would invent a value no device reported and no dose was based on.
pub fn debounce_cgm(records: Vec<Record>) -> (Vec<Record>, usize) {
    let mut seen = std::collections::HashSet::new();
    let mut kept = Vec::with_capacity(records.len());
    let mut dropped = 0;
    for r in records {
        if r.kind() != kind::CGM {
            kept.push(r);
            continue;
        }
        if seen.insert(r.t().div_euclid(CGM_BUCKET_MS)) {
            kept.push(r);
        } else {
            dropped += 1;
        }
    }
    (kept, dropped)
}

/// Drop records already emitted, by canonical content.
///
/// THIS IS THE ON-PHONE EQUIVALENT OF THE SNAPSHOT FILTERS, AND IT IS NOT THE
/// SAME OPERATION. `spec/records.md` §3 drops version rows because a snapshot
/// shows only final state. The AAPS sync queue does the opposite: it walks every
/// row including version rows, and resolves each one to the *current* record —
/// so a live emitter sees the same logical record again on every edit.
///
/// Measured on the reference snapshot: of 12,876 version rows, 12,874 resolve to
/// something byte-identical to what was already emitted. This settles those.
/// What it cannot settle is a genuine edit arriving after its epoch was sealed,
/// which is roughly fifteen a year and is spec §7's reserved `amend` question.
#[derive(Default)]
pub struct Emitted {
    seen: std::collections::HashSet<String>,
    /// Records that differed from something already emitted for the same
    /// `(t, k)` — i.e. real edits. Counted rather than handled, on purpose.
    pub amendments: usize,
}

impl Emitted {
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns true if this record is new and should be emitted.
    pub fn accept(&mut self, record: &Record) -> bool {
        let json = record.to_canonical_json();
        if !self.seen.insert(json) {
            return false;
        }
        // A second, different record at the same instant and kind is an edit,
        // not a duplicate. Count it; spec §7 says do not guess a mechanism yet.
        let key = format!("{}\u{1}{}", record.t(), record.kind());
        if !self.seen.insert(key) {
            self.amendments += 1;
        }
        true
    }
}

/// Encode a full stream: the header, then the records, one JSON object per line.
pub fn encode(records: &[Record]) -> String {
    let mut out = String::new();
    out.push_str(&header().to_canonical_json());
    out.push('\n');
    for r in records {
        out.push_str(&r.to_canonical_json());
        out.push('\n');
    }
    out
}
