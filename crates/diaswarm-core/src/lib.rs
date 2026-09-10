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

pub mod invite;
pub mod seal;
pub mod vault;

use std::collections::BTreeMap;

use serde_json::{Map, Value};

/// The version of `spec/records.md` this implements. Declared in the header.
pub const SPEC_VERSION: u64 = 3;

/// A day's worth of milliseconds. The width of an epoch, not its phase.
pub const EPOCH_MS: i64 = 24 * 60 * 60 * 1000;

/// How epochs are cut: a day, shifted by a fixed per-subject offset.
///
/// WHY NOT PLAIN UTC, WHICH THIS USED TO BE. UTC keeps epoch identity
/// unambiguous — a constant, so a record maps to the same epoch wherever the
/// phone is — and that argument is right and is preserved here, because a fixed
/// offset is still a constant. What UTC got wrong was the PHASE. At UTC+12 a
/// UTC epoch runs local noon to local noon, so one local day is split 12 hours
/// either side of two epochs, and "share yesterday" shares two half-days.
///
/// Worse, revocation takes effect at the next boundary, so the worst case —
/// nearly a full day retained — lands just after local noon, in the middle of
/// the waking day. Shifted to local midnight the worst case lands while the
/// subject is asleep, which is when least happens.
///
/// This is NOT local time. The offset is a fixed constant recorded once, so it
/// does not follow DST and does not move when the subject travels: a record's
/// epoch never depends on where it was written.
pub const EPOCH_BASIS: &str = "offset-day";

/// The offset to use when nothing better is known. Zero is plain UTC, which is
/// what v2 did, so an unconfigured emitter behaves as before rather than
/// silently choosing a phase for someone.
pub const DEFAULT_EPOCH_OFFSET_MS: i64 = 0;

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

    /// Which epoch this record falls in, under a given offset.
    pub fn epoch(&self, offset_ms: i64) -> i64 {
        epoch_of(self.t(), offset_ms)
    }

    /// Apply the spec's precision rules. Idempotent, and asserted so on real
    /// data: normalising an already-canonical stream must not move a single byte.
    pub fn normalise(mut self) -> Self {
        let kind = self.kind().to_string();

        // Profile blocks first: the unit that decides the scale is removed by
        // the same step that applies it, so a normalised record never carries
        // one and an un-normalisable record always does.
        if kind == kind::PROFILE {
            let unit = self.0.get("unit").and_then(Value::as_str).map(str::to_string);
            let (scale, recognised) = profile_scale(unit.as_deref());
            if recognised {
                if let Some(isf) = self.0.get_mut("isf") {
                    scale_blocks(isf, scale, &["amount"]);
                }
                if let Some(target) = self.0.get_mut("target") {
                    scale_blocks(target, scale, &["lowTarget", "highTarget"]);
                }
                self.0.remove("unit");
            }
        }

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
/// AAPS's own constant (`Constants.MMOLL_TO_MGDL`), not the textbook 18.
///
/// Profile blocks are stored in whichever unit the user set, so converting with
/// the constant the loop itself used is what reproduces the numbers it dosed on.
pub const MMOLL_TO_MGDL: f64 = 18.0182;

/// How to get a profile's glucose-bearing blocks into mg/dL.
///
/// THE HAZARD. `cgm.mgdl` and `target.lo` are mg/dL always, but profile blocks
/// are in the user's own unit. Emitting both untouched puts a target of 160.2
/// and a target of 5 in one stream meaning nearly the same thing — a factor of
/// eighteen apart — which is exactly what §2 forbids. An unrecognised unit is
/// left alone AND kept visible, so a consumer meets "not normalised" rather than
/// a plausible wrong number.
fn profile_scale(unit: Option<&str>) -> (f64, bool) {
    match unit.map(|u| u.trim().to_ascii_uppercase()).as_deref() {
        Some("MGDL") | Some("MG/DL") => (1.0, true),
        Some("MMOL") | Some("MMOLL") | Some("MMOL/L") => (MMOLL_TO_MGDL, true),
        _ => (1.0, false),
    }
}

fn scale_blocks(value: &mut Value, scale: f64, fields: &[&str]) {
    let Some(blocks) = value.as_array_mut() else { return };
    for b in blocks {
        let Some(obj) = b.as_object_mut() else { continue };
        for f in fields {
            if let Some(n) = obj.get(*f).and_then(Value::as_f64) {
                if let Some(r) = serde_json::Number::from_f64(round_half_even(n * scale, 1)) {
                    obj.insert((*f).to_string(), Value::Number(r));
                }
            }
        }
    }
}

fn round_half_even(value: f64, digits: i32) -> f64 {
    let factor = 10f64.powi(digits);
    (value * factor).round_ties_even() / factor
}

/// Which epoch a timestamp falls in.
///
/// `div_euclid`, not `/`: a plain divide truncates toward zero, so timestamps
/// either side of the offset origin would share an epoch index.
pub fn epoch_of(t: i64, offset_ms: i64) -> i64 {
    (t + offset_ms).div_euclid(EPOCH_MS)
}

/// The stream header (spec §5.2). The only record that is not an event.
///
/// Carries the epoch offset because a consumer cannot infer it: the same
/// records cut at a different phase are a different set of days, and getting
/// that wrong silently reshapes every daily figure computed from them.
pub fn header(offset_ms: i64) -> Record {
    Record::new(0, kind::META)
        .set("spec", Some(Value::from(SPEC_VERSION)))
        .set("epoch", Some(Value::from(EPOCH_BASIS)))
        .set("offset", Some(Value::from(offset_ms)))
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
    /// The newest five-minute bucket a CGM reading has been emitted for.
    ///
    /// **CARRIED ACROSS RUNS BY THE CALLER**, which is the whole difficulty. A
    /// set of seen buckets works within one drain and is useless live: readings
    /// arrive one per minute in separate passes, so an in-memory set never sees
    /// two readings from the same bucket together. A high-water mark survives
    /// because it is one number the caller can persist beside its other ones.
    last_cgm_bucket: Option<i64>,
    /// CGM readings dropped for sharing a bucket with an earlier one.
    pub thinned: usize,
}

impl Emitted {
    pub fn new() -> Self {
        Self::default()
    }

    /// Resume, knowing the newest CGM bucket already emitted.
    ///
    /// Pass `None` on a first run or after a reset; anything else and the first
    /// reading of each bucket since is what gets through.
    pub fn resuming(last_cgm_bucket: Option<i64>) -> Self {
        Self { last_cgm_bucket, ..Self::default() }
    }

    /// The newest CGM bucket emitted so far, for the caller to persist.
    pub fn last_cgm_bucket(&self) -> Option<i64> {
        self.last_cgm_bucket
    }

    /// Returns true if this record is new and should be emitted.
    pub fn accept(&mut self, record: &Record) -> bool {
        // ONE CGM READING PER FIVE MINUTES, KEEPING THE FIRST (spec §3.3).
        //
        // Not a duplicate filter. This subject's Libre 3 reports every 59
        // seconds — 1,586 readings a day where the Dexcom it replaced managed
        // 255 — and every one of them is a real, distinct value the loop acted
        // on. They are thinned because a *follower* is not disadvantaged by
        // getting one in five, while carrying all of them costs every peer in
        // the pool 51.5 MB a year of CGM instead of 8.3.
        //
        // The loop is unaffected: it reads the database, not this stream.
        if record.kind() == kind::CGM {
            let bucket = record.t().div_euclid(CGM_BUCKET_MS);
            match self.last_cgm_bucket {
                Some(last) if bucket <= last => {
                    self.thinned += 1;
                    return false;
                }
                _ => self.last_cgm_bucket = Some(bucket),
            }
        }

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
/// Just the records, no header. For appending to a stream that has one.
pub fn encode_records(records: &[Record]) -> String {
    let mut out = String::new();
    for r in records {
        out.push_str(&r.to_canonical_json());
        out.push('\n');
    }
    out
}

pub fn encode(records: &[Record], offset_ms: i64) -> String {
    let mut out = String::new();
    out.push_str(&header(offset_ms).to_canonical_json());
    out.push('\n');
    for r in records {
        out.push_str(&r.to_canonical_json());
        out.push('\n');
    }
    out
}
