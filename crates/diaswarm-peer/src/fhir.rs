//! The digest as a FHIR bundle an institution can actually ingest.
//!
//! **[feasibility §10.7](../../../docs/feasibility.md) DECIDED THIS, AND IT IS
//! THE POINT OF CHOOSING FHIR AT ALL:** *"It is the difference between a
//! clinician being handed a file and a clinician being handed a file their
//! system ingests."* A hand-rolled JSON shape would defeat the entire reason
//! for the format, so this follows HL7's published profiles rather than
//! inventing one.
//!
//! **HL7 FHIR Implementation Guide: Continuous Glucose Monitoring, v1.0.0**
//! (STU 1, published 2025-09), `http://hl7.org/fhir/uv/cgm/`. The codes below
//! are read from its StructureDefinitions, not from memory:
//!
//! | resource | LOINC |
//! |---|---|
//! | CGM Summary (grouping) | `107931-8` |
//! | Times in Ranges | `106793-3` |
//! | ↳ very low / low / target / high / very high | `104642-4` / `104641-6` / `97510-2` / `104640-8` / `104639-0` |
//! | Mean glucose (mass) | `97507-8` |
//! | Glucose Management Indicator | `97506-0` |
//! | Coefficient of variation | `104638-2` |
//! | Days of wear | `104636-6` |
//! | Sensor active percentage | `104637-4` |
//!
//! The grouping observation requires `hasMember` for times-in-ranges, GMI, CV,
//! days of wear and sensor active percentage; mean glucose is optional and is
//! always included here because a clinic expects it.
//!
//! **COMPUTED AT THE READER, WHICH IS §10.7'S WHOLE DESIGN POINT.** Nothing new
//! is published, no plaintext leaves anyone's phone on the way to a format, and
//! the subject's revocation still governs — a reader whose grant stops gets no
//! new segments to summarise. An exporter that ran anywhere else would be a
//! custodian.
//!
//! ✅ **VALIDATED, LOCALLY, WITH HL7'S OWN VALIDATOR.** `validator_cli.jar`
//! against `hl7.fhir.uv.cgm#1.0.0` on FHIR R4. The first attempt failed with 32
//! errors in two classes, both fixed here and both worth recording because
//! neither is visible from reading the spec:
//!
//! 1. **Every `display` was wrong.** I had used each profile's *description*
//!    ("Mean glucose from a continuous glucose monitoring (CGM) summary") where
//!    FHIR wants the *LOINC display* ("Average glucose [Mass/volume] in
//!    Interstitial fluid during Reporting Period"). A wrong display is an
//!    error; **an absent one is not**, so display is now omitted entirely. That
//!    is also version-proof: pinning LOINC long-names into this file would tie
//!    the exporter to a LOINC release it does not ship and cannot re-check.
//! 2. **Entries had no `fullUrl`**, so every `subject` and `hasMember`
//!    reference was an unresolvable relative reference — one error for the
//!    missing `fullUrl` and one for each reference that depended on it.
//!
//! **Result after the fixes: `Success: 0 errors, 14 warnings, 0 notes`.**
//!
//! The 14 warnings are both Best Practice Recommendations and both are left:
//!
//! * `dom-6`, "a resource should have narrative" (7). Narrative is a
//!   human-readable rendering of each resource, and adding seven of them would
//!   roughly double a bundle whose entire purpose is to be *small*. The
//!   receiving system renders these.
//! * "all observations should have a performer" (7). **This gateway does not
//!   know one, and inventing one would be a lie about provenance.** Nobody
//!   performed these observations in the FHIR sense: a sensor produced
//!   readings, a subject sealed them, and a reader computed statistics. The
//!   honest answer is absence.
//!
//! ⚠️ **Local validation is not a server accepting it.** §10.7 reopens its
//! interoperability verdict on "one FHIR server validating a bundle"; this is
//! the reference validator agreeing it conforms, which is necessary and not
//! quite the same thing.

use crate::agp::Agp;

const LOINC: &str = "http://loinc.org";
const UCUM: &str = "http://unitsofmeasure.org";
const IG: &str = "http://hl7.org/fhir/uv/cgm/StructureDefinition";

fn esc(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

/// `YYYY-MM-DD` from epoch milliseconds, UTC.
///
/// The CGM profiles fix `effectivePeriod` to date precision, so this is the
/// format they ask for rather than a shortening of a timestamp.
fn ymd(ms: i64) -> String {
    let days = ms.div_euclid(86_400_000);
    // Civil-from-days, Howard Hinnant's algorithm — the one every date library
    // uses, written out because pulling a date crate for two functions is not
    // the trade this binary should make.
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
    format!("{y:04}-{m:02}-{d:02}")
}

fn quantity(value: f64, unit: &str, code: &str) -> String {
    format!(
        r#"{{"value":{value:.2},"unit":"{}","system":"{UCUM}","code":"{}"}}"#,
        esc(unit),
        esc(code)
    )
}

/// A deterministic `urn:uuid` for one resource in one report.
///
/// **DETERMINISTIC ON PURPOSE.** A transaction bundle needs a `fullUrl` per
/// entry for its internal references to resolve, and deriving it from the
/// content means re-running the same export produces the same bundle — so a
/// resubmission is recognisably the same document rather than a duplicate.
///
/// Shaped as a v4 UUID (version and variant bits set) because `urn:uuid:` is
/// specified to hold one, not because the bytes are random.
fn urn_uuid(seed: &str) -> String {
    let h = p2panda_core::Hash::digest(seed.as_bytes());
    let b = h.as_bytes();
    let mut u = [0u8; 16];
    u.copy_from_slice(&b[..16]);
    u[6] = (u[6] & 0x0f) | 0x40;
    u[8] = (u[8] & 0x3f) | 0x80;
    let hex: String = u.iter().map(|x| format!("{x:02x}")).collect();
    format!(
        "urn:uuid:{}-{}-{}-{}-{}",
        &hex[0..8], &hex[8..12], &hex[12..16], &hex[16..20], &hex[20..32]
    )
}

/// **NO `display`, DELIBERATELY.** See the header: a wrong LOINC display is a
/// validation error and an absent one is not, and the long names drift with
/// LOINC releases this crate does not ship.
fn observation(
    id: &str,
    full_url: &str,
    profile: &str,
    loinc: &str,
    subject: &str,
    start: &str,
    end: &str,
    body: &str,
) -> String {
    format!(
        r#"{{"fullUrl":"{full_url}","resource":{{"resourceType":"Observation","id":"{id}","meta":{{"profile":["{IG}/{profile}"]}},"status":"final","code":{{"coding":[{{"system":"{LOINC}","code":"{loinc}"}}]}},"subject":{{"reference":"Patient/{}"}},"effectivePeriod":{{"start":"{start}","end":"{end}"}},{body}}},"request":{{"method":"POST","url":"Observation"}}}}"#,
        esc(subject)
    )
}

/// The whole digest as one transaction bundle.
///
/// `subject_id` is the Patient this is about. **It is the caller's to choose
/// and this module will not invent one**: a gateway that minted patient
/// identifiers would be the identity broker D5 forbids.
pub fn cgm_summary_bundle(agp: &Agp, subject_id: &str) -> String {
    let start = ymd(agp.first_t);
    let end = ymd(agp.last_t);
    let s = subject_id;

    let tir_component = |loinc: &str, pct: f64| {
        format!(
            r#"{{"code":{{"coding":[{{"system":"{LOINC}","code":"{loinc}"}}]}},"valueQuantity":{}}}"#,
            quantity(pct, "%", "%")
        )
    };

    // One urn:uuid per resource, derived from the report it belongs to so the
    // same export is the same document.
    let stamp = format!("{s}|{start}|{end}");
    let url = |id: &str| urn_uuid(&format!("{stamp}|{id}"));
    let (u_summary, u_ranges, u_mean, u_gmi, u_cv, u_wear, u_active) = (
        url("cgm-summary"), url("times-in-ranges"), url("mean-glucose"),
        url("gmi"), url("cv"), url("days-of-wear"), url("sensor-active"),
    );

    let ranges = observation(
        "times-in-ranges", &u_ranges, "cgm-summary-times-in-ranges", "106793-3",
        s, &start, &end,
        &format!(
            r#""component":[{}]"#,
            [
                tir_component("104642-4", agp.very_low_percent),
                tir_component("104641-6", agp.low_percent),
                tir_component("97510-2", agp.in_range_percent),
                tir_component("104640-8", agp.high_percent),
                tir_component("104639-0", agp.very_high_percent),
            ]
            .join(",")
        ),
    );

    let mean = observation(
        "mean-glucose", &u_mean, "cgm-summary-mean-glucose-mass-per-volume", "97507-8",
        s, &start, &end,
        &format!(r#""valueQuantity":{}"#, quantity(agp.mean_mgdl, "mg/dl", "mg/dL")),
    );
    let gmi = observation(
        "gmi", &u_gmi, "cgm-summary-gmi", "97506-0", s, &start, &end,
        &format!(r#""valueQuantity":{}"#, quantity(agp.gmi_percent, "%", "%")),
    );
    let cv = observation(
        "cv", &u_cv, "cgm-summary-coefficient-of-variation", "104638-2", s, &start, &end,
        &format!(r#""valueQuantity":{}"#, quantity(agp.cv_percent, "%", "%")),
    );
    let wear = observation(
        "days-of-wear", &u_wear, "cgm-summary-days-of-wear", "104636-6", s, &start, &end,
        &format!(r#""valueQuantity":{}"#, quantity(agp.days_of_wear, "days", "d")),
    );
    let active = observation(
        "sensor-active", &u_active, "cgm-summary-sensor-active-percentage", "104637-4",
        s, &start, &end,
        &format!(r#""valueQuantity":{}"#, quantity(agp.sensor_active_percent, "%", "%")),
    );

    // **hasMember POINTS AT THE fullUrls, NOT AT `Observation/<id>`.** A
    // relative reference inside a bundle whose entries have no fullUrl does not
    // resolve, which the validator reports once for the missing fullUrl and
    // again for every reference that depended on it.
    let members = [&u_ranges, &u_mean, &u_gmi, &u_cv, &u_wear, &u_active]
        .iter()
        .map(|u| format!(r#"{{"reference":"{u}"}}"#))
        .collect::<Vec<_>>()
        .join(",");
    let summary = observation(
        "cgm-summary", &u_summary, "cgm-summary", "107931-8", s, &start, &end,
        &format!(r#""hasMember":[{members}]"#),
    );

    format!(
        r#"{{"resourceType":"Bundle","type":"transaction","entry":[{}]}}"#,
        [summary, ranges, mean, gmi, cv, wear, active].join(",")
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agp::Agp;

    fn sample() -> Agp {
        Agp {
            readings: 1000,
            days_of_wear: 90.0,
            sensor_active_percent: 96.5,
            mean_mgdl: 150.0,
            cv_percent: 32.4,
            gmi_percent: 6.898,
            very_low_percent: 0.5,
            low_percent: 2.5,
            in_range_percent: 72.0,
            high_percent: 20.0,
            very_high_percent: 5.0,
            cadence_seconds: 60,
            first_t: 1_735_603_200_000, // 2024-12-31
            last_t: 1_789_516_800_000,  // 2026-09-16
        }
    }

    /// CIVIL DATES, BECAUSE THE PROFILES FIX `effectivePeriod` TO DATE
    /// PRECISION AND A WRONG DATE IS A WRONG REPORT.
    #[test]
    fn dates_are_real_civil_dates_including_a_leap_day() {
        assert_eq!(ymd(0), "1970-01-01");
        assert_eq!(ymd(1_789_516_800_000), "2026-09-16");
        assert_eq!(ymd(951_782_400_000), "2000-02-29", "the 2000 leap day is wrong");
        assert_eq!(ymd(1_735_603_200_000), "2024-12-31");
    }

    /// A BUNDLE THAT IS NOT VALID JSON IS NOT A BUNDLE.
    #[test]
    fn the_bundle_parses_and_carries_the_igs_codes() {
        let json = cgm_summary_bundle(&sample(), "patient-123");
        let v: serde_json::Value = serde_json::from_str(&json).expect("the bundle is not valid JSON");

        assert_eq!(v["resourceType"], "Bundle");
        assert_eq!(v["type"], "transaction");
        let entries = v["entry"].as_array().expect("entries");
        assert_eq!(entries.len(), 7, "expected a grouping observation and six metrics");

        // Every LOINC code the IG fixes, read off the StructureDefinitions.
        let codes: Vec<String> = entries
            .iter()
            .map(|e| e["resource"]["code"]["coding"][0]["code"].as_str().unwrap().to_string())
            .collect();
        for expected in
            ["107931-8", "106793-3", "97507-8", "97506-0", "104638-2", "104636-6", "104637-4"]
        {
            assert!(codes.contains(&expected.to_string()), "missing LOINC {expected}: {codes:?}");
        }

        // **EVERY ENTRY NEEDS A fullUrl**, or its references do not resolve —
        // 20 of the validator's 32 first-run errors were this and its knock-ons.
        let urls: Vec<&str> = entries.iter().map(|e| e["fullUrl"].as_str().expect("fullUrl")).collect();
        assert_eq!(urls.len(), 7);
        for u in &urls {
            assert!(u.starts_with("urn:uuid:"), "fullUrl is not a urn:uuid: {u}");
            assert_eq!(u.len(), 45, "not a UUID shape: {u}");
        }
        let unique: std::collections::BTreeSet<&&str> = urls.iter().collect();
        assert_eq!(unique.len(), 7, "two entries share a fullUrl");

        // The grouping observation must reference the others BY THEIR fullUrls,
        // or a server receives six unrelated observations.
        let summary = entries
            .iter()
            .find(|e| e["resource"]["id"] == "cgm-summary")
            .expect("no grouping observation");
        let members = summary["resource"]["hasMember"].as_array().expect("hasMember");
        assert_eq!(members.len(), 6, "the summary does not group all six metrics");
        for m in members {
            let r = m["reference"].as_str().unwrap();
            assert!(urls.contains(&r), "hasMember points at something not in the bundle: {r}");
        }

        // **NO display, DELIBERATELY** — a wrong LOINC display is a validation
        // error and every one of ours was wrong on the first run.
        for e in entries {
            assert!(
                e["resource"]["code"]["coding"][0]["display"].is_null(),
                "a display name came back; those drift with LOINC and must stay out"
            );
        }

        // Times in ranges carries five components, one per band.
        let ranges = entries
            .iter()
            .find(|e| e["resource"]["id"] == "times-in-ranges")
            .expect("no times-in-ranges");
        let components = ranges["resource"]["component"].as_array().expect("components");
        assert_eq!(components.len(), 5);
        let band_codes: Vec<&str> =
            components.iter().map(|c| c["code"]["coding"][0]["code"].as_str().unwrap()).collect();
        assert_eq!(band_codes, ["104642-4", "104641-6", "97510-2", "104640-8", "104639-0"]);

        // The same report twice is the same document, so a resubmission is not
        // a duplicate.
        let again = cgm_summary_bundle(&sample(), "patient-123");
        assert_eq!(again, json, "the bundle is not deterministic");

        // Units are the ones the profiles fix, not the ones that read nicely.
        let mean = entries.iter().find(|e| e["resource"]["id"] == "mean-glucose").unwrap();
        assert_eq!(mean["resource"]["valueQuantity"]["code"], "mg/dL");
        assert_eq!(mean["resource"]["valueQuantity"]["unit"], "mg/dl");
        let wear = entries.iter().find(|e| e["resource"]["id"] == "days-of-wear").unwrap();
        assert_eq!(wear["resource"]["valueQuantity"]["code"], "d");

        // Every observation is about the patient the caller named, and covers
        // the period the data actually covers.
        for e in entries {
            assert_eq!(e["resource"]["subject"]["reference"], "Patient/patient-123");
            assert_eq!(e["resource"]["effectivePeriod"]["start"], "2024-12-31");
            assert_eq!(e["resource"]["effectivePeriod"]["end"], "2026-09-16");
            assert_eq!(e["resource"]["status"], "final");
        }
    }

    /// A PATIENT ID IS CALLER-SUPPLIED AND MUST NOT BREAK THE DOCUMENT.
    #[test]
    fn a_hostile_patient_id_cannot_break_out_of_the_json() {
        let json = cgm_summary_bundle(&sample(), r#"x","evil":"1"#);
        let v: serde_json::Value = serde_json::from_str(&json).expect("quoting failed");
        assert!(v["entry"][0]["resource"]["evil"].is_null(), "an injected field survived");
    }
}
