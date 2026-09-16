//! The other half of [feasibility §10.7](../../../docs/feasibility.md): records
//! back out in the shape the diabetes ecosystem already speaks.
//!
//! **THE DIG_IT RECOMMENDATION APPLIED LITERALLY — build on what people already
//! use.** §10.7: *"the cheapest possible bridge to every follower app, watch
//! face and clinic dashboard that §11's comparison table currently scores as a
//! straight loss."* A clinician gets FHIR ([`crate::fhir`]); everyone else in
//! this ecosystem gets `entries` and `treatments`.
//!
//! **THE FIELD NAMES ARE READ OUT OF AAPS, NOT OUT OF MEMORY.** The producer and
//! consumer on the other side of this bridge is AndroidAPS, so the names come
//! from its own Nightscout SDK — `core/nssdk/.../RemoteEntry.kt` and
//! `RemoteTreatment.kt` — and the vocabularies from `core/data/.../TE.kt` and
//! `TrendArrow.kt`.
//!
//! 🔴 **THREE CONVERSIONS THAT A PASSTHROUGH WOULD GET SILENTLY WRONG**, and
//! each produces output that looks plausible and is not:
//!
//! 1. **`trend` is an enum NAME, Nightscout wants the TEXT.** The plugin writes
//!    `trendArrow.name`, so a record carries `FORTY_FIVE_UP`; Nightscout reads
//!    `FortyFiveUp`. Passing it through yields an arrow nothing renders.
//! 2. **`type` on an `event` is likewise an enum name** — `CANNULA_CHANGE`
//!    where Nightscout expects `Site Change`.
//! 3. **`dur` is MILLISECONDS and Nightscout's `duration` is MINUTES.** AAPS
//!    settles it: `timestamp..timestamp + duration`. A passthrough turns a
//!    30-minute temp basal into one lasting 1,800,000 minutes.
//! 4. **A temp target's `why` is an enum name too** — `HYPOGLYCEMIA` where
//!    Nightscout's `reason` reads `Hypo`.
//!
//! 🔴 **AND TWO THINGS THAT WOULD CORRUPT AN INSULIN TOTAL**, which matter more
//! than any of the above because every consumer sums `insulin`:
//!
//! * **A PRIMING bolus never reached the patient.** AAPS's own code says so and
//!   filters it out of IOB and TDD (`HovorkaMpcPlugin`, `AutotuneIob`). Exported
//!   as `insulin` it would inflate every total downstream, so it is excluded —
//!   and counted, because a silently shorter answer is the failure this project
//!   keeps being bitten by.
//! * **Insulin delivered as basal is not a bolus.** Nightscout has
//!   `isBasalInsulin` for exactly this; dropping the flag would double-count it
//!   against the basal rate a consumer already knows about.
//!
//! `isSMB` is carried for the same reason: the loop's own micro-boluses are
//! distinguishable in Nightscout and flattening them loses that.

use diaswarm_core::Record;

/// `TrendArrow` enum name → the text Nightscout renders.
fn direction(name: &str) -> &str {
    match name {
        "TRIPLE_UP" => "TripleUp",
        "DOUBLE_UP" => "DoubleUp",
        "SINGLE_UP" => "SingleUp",
        "FORTY_FIVE_UP" => "FortyFiveUp",
        "FLAT" => "Flat",
        "FORTY_FIVE_DOWN" => "FortyFiveDown",
        "SINGLE_DOWN" => "SingleDown",
        "DOUBLE_DOWN" => "DoubleDown",
        "TRIPLE_DOWN" => "TripleDown",
        // **"NONE" IS NIGHTSCOUT'S OWN VALUE FOR "NO ARROW"**, so an unknown
        // name degrades to something the ecosystem already handles rather than
        // to an invented string.
        _ => "NONE",
    }
}

/// `TT.Reason` enum name → the text Nightscout's `reason` carries.
fn target_reason(name: &str) -> &str {
    match name {
        "CUSTOM" => "Custom",
        "HYPOGLYCEMIA" => "Hypo",
        "ACTIVITY" => "Activity",
        "EATING_SOON" => "Eating Soon",
        "AUTOMATION" => "Automation",
        "WEAR" => "Wear",
        // Unknown reasons pass through: `reason` is free text in Nightscout, so
        // an unmapped name is ugly rather than wrong, and dropping it would
        // lose why a target was set.
        other => other,
    }
}

/// `TE.Type` enum name → the Nightscout `eventType`.
fn event_type(name: &str) -> Option<&'static str> {
    Some(match name {
        "CANNULA_CHANGE" => "Site Change",
        "INSULIN_CHANGE" => "Insulin Change",
        "PUMP_BATTERY_CHANGE" => "Pump Battery Change",
        "SENSOR_CHANGE" => "Sensor Change",
        "SENSOR_STARTED" => "Sensor Start",
        "SENSOR_STOPPED" => "Sensor Stop",
        "FINGER_STICK_BG_VALUE" => "BG Check",
        "EXERCISE" => "Exercise",
        "ANNOUNCEMENT" => "Announcement",
        "QUESTION" => "Question",
        "NOTE" => "Note",
        "APS_OFFLINE" => "OpenAPS Offline",
        "DAD_ALERT" => "D.A.D. Alert",
        "TUBE_CHANGE" => "Tube Change",
        // **AN UNKNOWN EVENT IS DROPPED, NOT GUESSED.** Nightscout's eventType
        // is a closed vocabulary; inventing a value would have every consumer
        // ignore the record anyway, and a wrong one could be rendered as
        // something it is not.
        _ => return None,
    })
}

fn esc(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"").replace('\n', "\\n")
}

/// ISO 8601 in UTC, which is what `dateString` and `created_at` carry.
pub fn iso8601(ms: i64) -> String {
    let days = ms.div_euclid(86_400_000);
    let rem = ms.rem_euclid(86_400_000);
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    let (h, min, sec, milli) =
        (rem / 3_600_000, (rem / 60_000) % 60, (rem / 1000) % 60, rem % 1000);
    format!("{y:04}-{m:02}-{d:02}T{h:02}:{min:02}:{sec:02}.{milli:03}Z")
}

fn num(r: &Record, key: &str) -> Option<f64> {
    r.get(key)?.as_f64()
}

/// Milliseconds to whole minutes, which is the unit `duration` is in.
fn minutes(ms: f64) -> f64 {
    (ms / 60_000.0).round()
}

/// `cgm` records as Nightscout `entries`.
pub fn entries(records: &[Record]) -> Vec<String> {
    records
        .iter()
        .filter(|r| r.kind() == "cgm")
        .filter_map(|r| {
            let sgv = num(r, "mgdl")?;
            let mut fields = vec![
                r#""type":"sgv""#.to_string(),
                format!(r#""sgv":{sgv}"#),
                format!(r#""date":{}"#, r.t()),
                format!(r#""dateString":"{}""#, iso8601(r.t())),
                r#""units":"mg/dl""#.to_string(),
            ];
            if let Some(t) = r.get("trend").and_then(|v| v.as_str()) {
                fields.push(format!(r#""direction":"{}""#, direction(t)));
            }
            if let Some(src) = r.get("src").and_then(|v| v.as_str()) {
                fields.push(format!(r#""device":"{}""#, esc(src)));
            }
            Some(format!("{{{}}}", fields.join(",")))
        })
        .collect()
}

/// Everything that is not a reading, as Nightscout `treatments`.
///
/// **`meta` IS NOT AN EVENT** and never becomes a treatment — it is the stream
/// header (spec §5.2).
pub fn treatments(records: &[Record]) -> Vec<String> {
    records
        .iter()
        .filter_map(|r| {
            let t = r.t();
            let base = |event: &str| {
                vec![
                    format!(r#""eventType":"{event}""#),
                    format!(r#""date":{t}"#),
                    format!(r#""created_at":"{}""#, iso8601(t)),
                ]
            };
            let mut f = match r.kind() {
                "bolus" => {
                    // **PRIMING NEVER REACHED THE PATIENT.** Counted by the
                    // caller, not silently dropped.
                    if r.get("type").and_then(|v| v.as_str()) == Some("PRIMING") {
                        return None;
                    }
                    // **"Correction Bolus" RATHER THAN "Meal Bolus"** because a
                    // record does not say whether carbs accompanied it — carbs
                    // are their own record here. Both sum insulin identically;
                    // the difference is how a dashboard labels it.
                    let mut f = base("Correction Bolus");
                    f.push(format!(r#""insulin":{}"#, num(r, "u")?));
                    if r.get("type").and_then(|v| v.as_str()) == Some("SMB") {
                        f.push(r#""isSMB":true"#.to_string());
                    }
                    if r.get("basal").and_then(|v| v.as_bool()) == Some(true) {
                        f.push(r#""isBasalInsulin":true"#.to_string());
                    }
                    f
                }
                "carb" => {
                    let mut f = base("Carb Correction");
                    f.push(format!(r#""carbs":{}"#, num(r, "g")?));
                    f
                }
                "tbr" => {
                    let mut f = base("Temp Basal");
                    let rate = num(r, "rate")?;
                    // **`abs` SAYS WHICH FIELD THE RATE IS.** Nightscout keeps
                    // absolute (U/hr) and percent apart, and putting 180 in
                    // `absolute` would read as 180 units an hour.
                    match r.get("abs").and_then(|v| v.as_bool()) {
                        Some(true) => f.push(format!(r#""absolute":{rate}"#)),
                        _ => f.push(format!(r#""percent":{}"#, rate - 100.0)),
                    }
                    f
                }
                "extbolus" => {
                    let mut f = base("Combo Bolus");
                    f.push(format!(r#""insulin":{}"#, num(r, "u")?));
                    f
                }
                "target" => {
                    let mut f = base("Temporary Target");
                    if let Some(lo) = num(r, "lo") {
                        f.push(format!(r#""targetBottom":{lo}"#));
                    }
                    if let Some(hi) = num(r, "hi") {
                        f.push(format!(r#""targetTop":{hi}"#));
                    }
                    if let Some(why) = r.get("why").and_then(|v| v.as_str()) {
                        f.push(format!(r#""reason":"{}""#, esc(target_reason(why))));
                    }
                    f
                }
                "profile" => {
                    let mut f = base("Profile Switch");
                    if let Some(n) = r.get("name").and_then(|v| v.as_str()) {
                        f.push(format!(r#""profile":"{}""#, esc(n)));
                    }
                    if let Some(p) = num(r, "pct") {
                        f.push(format!(r#""percentage":{p}"#));
                    }
                    // Nightscout calls it `timeshift`; the record calls it
                    // `shift`. Hours, both sides.
                    if let Some(sh) = num(r, "shift") {
                        f.push(format!(r#""timeshift":{sh}"#));
                    }
                    f
                }
                "event" => {
                    let name = r.get("type").and_then(|v| v.as_str())?;
                    let mut f = base(event_type(name)?);
                    if let Some(g) = num(r, "mgdl") {
                        f.push(format!(r#""glucose":{g},"units":"mg/dl""#));
                    }
                    if let Some(note) = r.get("note").and_then(|v| v.as_str()) {
                        f.push(format!(r#""notes":"{}""#, esc(note)));
                    }
                    f
                }
                _ => return None,
            };
            // **MINUTES, NOT MILLISECONDS** — and both, because Nightscout
            // accepts `durationInMilliseconds` and consumers differ in which
            // they read.
            if let Some(ms) = num(r, "dur").filter(|d| *d > 0.0) {
                f.push(format!(r#""duration":{}"#, minutes(ms)));
                f.push(format!(r#""durationInMilliseconds":{ms}"#));
            }
            f.push(r#""enteredBy":"diaswarm""#.to_string());
            Some(format!("{{{}}}", f.join(",")))
        })
        .collect()
}

/// How many boluses were left out because they never reached the patient.
///
/// **SO THE CALLER CAN SAY SO.** `treatments` returning fewer records than it
/// was given is correct and must still be visible.
pub fn priming_excluded(records: &[Record]) -> usize {
    records
        .iter()
        .filter(|r| r.kind() == "bolus")
        .filter(|r| r.get("type").and_then(|v| v.as_str()) == Some("PRIMING"))
        .count()
}

/// A JSON array, which is what both endpoints take.
pub fn as_array(items: &[String]) -> String {
    format!("[{}]", items.join(","))
}

#[cfg(test)]
mod tests {
    use super::*;
    use diaswarm_core::Record;
    use serde_json::Value as J;

    fn parse(items: &[String]) -> Vec<J> {
        items.iter().map(|s| serde_json::from_str(s).expect("not JSON")).collect()
    }

    /// THE TREND IS AN ENUM NAME AND NIGHTSCOUT WANTS THE TEXT.
    ///
    /// `FORTY_FIVE_UP` passed through renders as no arrow at all in every
    /// consumer, which looks like missing data rather than a mapping bug.
    #[test]
    fn trend_names_become_nightscout_directions() {
        let r = vec![
            Record::new(1_700_000_000_000, "cgm")
                .set("mgdl", Some(120.0.into()))
                .set("trend", Some("FORTY_FIVE_UP".into())),
            Record::new(1_700_000_060_000, "cgm")
                .set("mgdl", Some(118.0.into()))
                .set("trend", Some("FLAT".into())),
            Record::new(1_700_000_120_000, "cgm")
                .set("mgdl", Some(110.0.into()))
                .set("trend", Some("DOUBLE_DOWN".into())),
            // Something this build has never heard of must not invent a value.
            Record::new(1_700_000_180_000, "cgm")
                .set("mgdl", Some(110.0.into()))
                .set("trend", Some("SEXTUPLE_SIDEWAYS".into())),
        ];
        let e = parse(&entries(&r));
        assert_eq!(e.len(), 4);
        assert_eq!(e[0]["direction"], "FortyFiveUp");
        assert_eq!(e[1]["direction"], "Flat");
        assert_eq!(e[2]["direction"], "DoubleDown");
        assert_eq!(e[3]["direction"], "NONE", "an unknown arrow was invented");
        // The rest of the entry shape, from AAPS's RemoteEntry.
        assert_eq!(e[0]["type"], "sgv");
        assert_eq!(e[0]["sgv"], 120.0);
        assert_eq!(e[0]["date"], 1_700_000_000_000i64);
        assert_eq!(e[0]["dateString"], "2023-11-14T22:13:20.000Z");
        assert_eq!(e[0]["units"], "mg/dl");
    }

    /// DURATION IS MILLISECONDS HERE AND MINUTES THERE.
    ///
    /// A passthrough turns a 30-minute temp basal into one lasting 1,800,000
    /// minutes — which a dashboard will happily draw.
    #[test]
    fn durations_are_converted_to_minutes() {
        let r = vec![
            Record::new(1_700_000_000_000, "tbr")
                .set("rate", Some(0.9.into()))
                .set("abs", Some(true.into()))
                .set("dur", Some(1_800_000.0.into())),
        ];
        let t = parse(&treatments(&r));
        assert_eq!(t.len(), 1);
        assert_eq!(t[0]["eventType"], "Temp Basal");
        assert_eq!(t[0]["duration"], 30.0, "duration is not in minutes");
        assert_eq!(t[0]["durationInMilliseconds"], 1_800_000.0);
        assert_eq!(t[0]["absolute"], 0.9);
        assert!(t[0]["percent"].is_null(), "an absolute rate was also sent as a percent");
    }

    /// A PERCENT RATE IS NOT AN ABSOLUTE ONE, AND 180 IN THE WRONG FIELD READS
    /// AS 180 UNITS AN HOUR.
    #[test]
    fn a_percentage_temp_basal_goes_in_the_percent_field() {
        let r = vec![
            Record::new(1_700_000_000_000, "tbr")
                .set("rate", Some(180.0.into()))
                .set("abs", Some(false.into()))
                .set("dur", Some(900_000.0.into())),
        ];
        let t = parse(&treatments(&r));
        assert!(t[0]["absolute"].is_null(), "a percentage was sent as units per hour");
        // Nightscout's `percent` is the change from basal, not the ratio.
        assert_eq!(t[0]["percent"], 80.0);
        assert_eq!(t[0]["duration"], 15.0);
    }

    /// EVENT TYPES ARE ENUM NAMES TOO, AND AN UNKNOWN ONE IS DROPPED.
    #[test]
    fn event_types_map_or_are_left_out() {
        let r = vec![
            Record::new(1_700_000_000_000, "event").set("type", Some("CANNULA_CHANGE".into())),
            Record::new(1_700_000_060_000, "event")
                .set("type", Some("FINGER_STICK_BG_VALUE".into()))
                .set("mgdl", Some(101.0.into())),
            Record::new(1_700_000_120_000, "event")
                .set("type", Some("NOTE".into()))
                .set("note", Some("felt low".into())),
            Record::new(1_700_000_180_000, "event").set("type", Some("SOMETHING_NEW".into())),
        ];
        let t = parse(&treatments(&r));
        assert_eq!(t.len(), 3, "an unknown event type was guessed at rather than dropped");
        assert_eq!(t[0]["eventType"], "Site Change");
        assert_eq!(t[1]["eventType"], "BG Check");
        assert_eq!(t[1]["glucose"], 101.0);
        assert_eq!(t[2]["eventType"], "Note");
        assert_eq!(t[2]["notes"], "felt low");
    }

    /// EVERY OTHER KIND LANDS IN THE RIGHT PLACE, AND `meta` IS NOT AN EVENT.
    #[test]
    fn kinds_are_split_between_entries_and_treatments() {
        let r = vec![
            Record::new(1, "cgm").set("mgdl", Some(100.0.into())),
            Record::new(2, "bolus").set("u", Some(1.5.into())),
            Record::new(3, "carb").set("g", Some(30.0.into())),
            Record::new(4, "extbolus").set("u", Some(2.0.into())).set("dur", Some(3_600_000.0.into())),
            Record::new(5, "target").set("lo", Some(80.0.into())).set("hi", Some(120.0.into())),
            Record::new(6, "profile").set("name", Some("Default".into())).set("pct", Some(110.0.into())),
            Record::new(7, "meta").set("spec", Some("6".into())),
        ];
        assert_eq!(entries(&r).len(), 1);
        let t = parse(&treatments(&r));
        let types: Vec<&str> = t.iter().map(|x| x["eventType"].as_str().unwrap()).collect();
        assert_eq!(
            types,
            ["Correction Bolus", "Carb Correction", "Combo Bolus", "Temporary Target", "Profile Switch"],
            "the stream header became a treatment, or a kind was lost"
        );
        assert_eq!(t[0]["insulin"], 1.5);
        assert_eq!(t[1]["carbs"], 30.0);
        assert_eq!(t[2]["duration"], 60.0);
        assert_eq!(t[3]["targetBottom"], 80.0);
        assert_eq!(t[3]["targetTop"], 120.0);
        assert_eq!(t[4]["profile"], "Default");
    }

    #[test]
    fn a_hostile_note_cannot_break_out_of_the_json() {
        let r = vec![
            Record::new(1, "event")
                .set("type", Some("NOTE".into()))
                .set("note", Some(r#"a" ,"eventType":"Site Change"#.into())),
        ];
        let t = parse(&treatments(&r));
        assert_eq!(t[0]["eventType"], "Note", "an injected field overrode the event type");
    }

    #[test]
    fn iso8601_is_a_real_timestamp() {
        assert_eq!(iso8601(0), "1970-01-01T00:00:00.000Z");
        assert_eq!(iso8601(951_782_400_000), "2000-02-29T00:00:00.000Z");
        assert_eq!(iso8601(1_789_516_800_123), "2026-09-16T00:00:00.123Z");
    }
}

#[cfg(test)]
mod compat_tests {
    use super::*;
    use diaswarm_core::Record;
    use serde_json::Value as J;

    fn parse(items: &[String]) -> Vec<J> {
        items.iter().map(|s| serde_json::from_str(s).expect("not JSON")).collect()
    }

    fn bolus(t: i64, u: f64, kind: &str, basal: bool) -> Record {
        Record::new(t, "bolus")
            .set("u", Some(u.into()))
            .set("type", Some(kind.into()))
            .set("basal", Some(basal.into()))
    }

    /// A PRIMING BOLUS NEVER REACHED THE PATIENT AND MUST NOT BE SUMMED.
    ///
    /// AAPS excludes it from IOB and TDD in its own code. Every Nightscout
    /// consumer adds up `insulin`, so exporting it inflates the total in
    /// software nobody here controls.
    #[test]
    fn priming_insulin_is_excluded_and_counted() {
        let r = vec![
            bolus(1, 1.5, "NORMAL", false),
            bolus(2, 0.3, "PRIMING", false),
            bolus(3, 0.7, "SMB", false),
        ];
        let t = parse(&treatments(&r));
        assert_eq!(t.len(), 2, "a priming bolus was exported as insulin");
        let total: f64 = t.iter().map(|x| x["insulin"].as_f64().unwrap()).sum();
        assert!((total - 2.2).abs() < 1e-9, "insulin total is {total}, not 2.2");
        assert_eq!(priming_excluded(&r), 1, "the exclusion was not countable");
    }

    /// SMB AND BASAL INSULIN ARE DISTINGUISHABLE IN NIGHTSCOUT, SO KEEP THEM.
    ///
    /// Flattening `isBasalInsulin` double-counts against a basal rate the
    /// consumer already knows about.
    #[test]
    fn smb_and_basal_insulin_keep_their_flags() {
        let r = vec![
            bolus(1, 1.5, "NORMAL", false),
            bolus(2, 0.7, "SMB", false),
            bolus(3, 0.2, "NORMAL", true),
        ];
        let t = parse(&treatments(&r));
        assert!(t[0]["isSMB"].is_null(), "a manual bolus was marked as an SMB");
        assert!(t[0]["isBasalInsulin"].is_null());
        assert_eq!(t[1]["isSMB"], true, "an SMB lost its flag");
        assert_eq!(t[2]["isBasalInsulin"], true, "basal insulin was exported as a plain bolus");
    }

    /// THE TEMP TARGET REASON IS AN ENUM NAME, LIKE EVERYTHING ELSE.
    #[test]
    fn target_reasons_become_nightscout_text() {
        let r = vec![
            Record::new(1, "target")
                .set("lo", Some(80.0.into()))
                .set("hi", Some(100.0.into()))
                .set("why", Some("HYPOGLYCEMIA".into())),
            Record::new(2, "target")
                .set("lo", Some(140.0.into()))
                .set("hi", Some(160.0.into()))
                .set("why", Some("EATING_SOON".into())),
            // Free text in Nightscout, so an unknown reason survives rather
            // than being dropped — ugly beats lost.
            Record::new(3, "target").set("lo", Some(90.0.into())).set("why", Some("SOMETHING".into())),
        ];
        let t = parse(&treatments(&r));
        assert_eq!(t[0]["reason"], "Hypo");
        assert_eq!(t[1]["reason"], "Eating Soon");
        assert_eq!(t[2]["reason"], "SOMETHING");
    }

    #[test]
    fn a_profile_switch_carries_its_timeshift() {
        let r = vec![
            Record::new(1, "profile")
                .set("name", Some("Default".into()))
                .set("pct", Some(110.0.into()))
                .set("shift", Some(2.0.into())),
        ];
        let t = parse(&treatments(&r));
        assert_eq!(t[0]["profile"], "Default");
        assert_eq!(t[0]["percentage"], 110.0);
        assert_eq!(t[0]["timeshift"], 2.0, "the timeshift was dropped");
    }
}
