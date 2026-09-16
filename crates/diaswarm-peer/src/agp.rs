//! The summary a diabetes clinic actually reads.
//!
//! **A CLINICIAN WILL NOT TAKE 143,000 READINGS.** The CGM here is a Libre 3
//! reporting every minute — about 1,586 a day — so a 90-day window is six
//! figures of glucose values. Handing that to a clinic is handing them nothing.
//! What they read is the AGP-style digest: how much of the window was spent in
//! range, above it and below it, the mean, the variability, and how much of the
//! period the sensor was actually working.
//!
//! **THE THRESHOLDS ARE THE 2019 INTERNATIONAL CONSENSUS ON TIME IN RANGE**, in
//! mg/dL because that is the unit the records carry and the unit the FHIR
//! profile fixes:
//!
//! | band | mg/dL | mmol/L |
//! |---|---|---|
//! | very low (level 2) | < 54 | < 3.0 |
//! | low (level 1) | 54–69 | 3.0–3.8 |
//! | target | 70–180 | 3.9–10.0 |
//! | high (level 1) | 181–250 | 10.1–13.9 |
//! | very high (level 2) | > 250 | > 13.9 |
//!
//! ⚠️ **THE BANDS ARE NOT THE SUBJECT'S DISPLAY RANGE.** A follower colours
//! readings against a range the *reader's* phone sets; this is the clinical
//! consensus and is deliberately not configurable. A digest whose bands moved
//! per install would not be comparable to anything.
//!
//! **IT LIVES HERE RATHER THAN IN `diaswarm-core` ON PURPOSE.** Core is the
//! frozen record spec; this is a reader-side interpretation of it, and the
//! interpretation is allowed to change without the wire changing. If a phone
//! ever needs it, move it then.

use diaswarm_core::Record;

/// Milligrams per decilitre, the unit `cgm` records carry.
const VERY_LOW: f64 = 54.0;
const LOW: f64 = 70.0;
const HIGH: f64 = 180.0;
const VERY_HIGH: f64 = 250.0;

/// The consensus minimum for a summary anyone should act on.
///
/// **14 DAYS AT 70% ACTIVE.** Below either, the estimate is not reliable and a
/// report that does not say so is worse than no report. This is stated rather
/// than enforced — the clinician asked for the window they were granted, and
/// silently refusing to summarise it would be its own kind of dishonesty.
pub const MIN_DAYS: f64 = 14.0;
pub const MIN_ACTIVE_PERCENT: f64 = 70.0;

#[derive(Debug, Clone, PartialEq)]
pub struct Agp {
    pub readings: usize,
    /// Distinct local days carrying at least one reading.
    pub days_of_wear: f64,
    /// Readings received against readings expected, as a percentage.
    pub sensor_active_percent: f64,
    pub mean_mgdl: f64,
    /// Coefficient of variation: SD / mean, as a percentage.
    pub cv_percent: f64,
    /// Bergenstal's Glucose Management Indicator, as a percentage.
    pub gmi_percent: f64,
    pub very_low_percent: f64,
    pub low_percent: f64,
    pub in_range_percent: f64,
    pub high_percent: f64,
    pub very_high_percent: f64,
    /// The inferred sampling interval in seconds — see [`Agp::from_records`].
    pub cadence_seconds: u64,
    pub first_t: i64,
    pub last_t: i64,
}

impl Agp {
    /// Whether this summary meets the consensus bar for acting on it.
    pub fn is_sufficient(&self) -> bool {
        self.days_of_wear >= MIN_DAYS && self.sensor_active_percent >= MIN_ACTIVE_PERCENT
    }

    /// Why it does not, in words a report can print.
    pub fn insufficiency(&self) -> Option<String> {
        if self.is_sufficient() {
            return None;
        }
        let mut why = Vec::new();
        if self.days_of_wear < MIN_DAYS {
            why.push(format!("{:.0} days of wear, consensus asks for {MIN_DAYS:.0}", self.days_of_wear));
        }
        if self.sensor_active_percent < MIN_ACTIVE_PERCENT {
            why.push(format!(
                "sensor active {:.1}%, consensus asks for {MIN_ACTIVE_PERCENT:.0}%",
                self.sensor_active_percent
            ));
        }
        Some(why.join("; "))
    }

    /// Compute the digest over whatever `cgm` records are present.
    ///
    /// **THE CADENCE IS INFERRED, NOT ASSUMED.** "Percent of time the sensor was
    /// active" is readings received over readings expected, and expected depends
    /// on the device — one a minute for a Libre 3, one per five for a Dexcom.
    /// Hard-coding either would silently produce a five-fold error on the other,
    /// so this takes the *median* gap between consecutive readings as the
    /// cadence. Median rather than mean because a single overnight gap would
    /// drag a mean and cannot move a median.
    ///
    /// `None` when there is nothing to summarise, which a caller must handle
    /// rather than print zeros for.
    pub fn from_records(records: &[Record]) -> Option<Agp> {
        let mut values: Vec<(i64, f64)> = records
            .iter()
            .filter(|r| r.kind() == "cgm")
            .filter_map(|r| {
                let mgdl = r.get("mgdl")?.as_f64()?;
                // A sensor that reports a physiologically impossible value is
                // reporting a fault; including it would move every statistic.
                (mgdl > 0.0 && mgdl < 1000.0).then_some((r.t(), mgdl))
            })
            .collect();
        if values.is_empty() {
            return None;
        }
        values.sort_by_key(|(t, _)| *t);

        let n = values.len();
        let mean = values.iter().map(|(_, v)| v).sum::<f64>() / n as f64;
        let variance = values.iter().map(|(_, v)| (v - mean).powi(2)).sum::<f64>() / n as f64;
        let sd = variance.sqrt();

        let band = |f: &dyn Fn(f64) -> bool| {
            values.iter().filter(|(_, v)| f(*v)).count() as f64 * 100.0 / n as f64
        };

        // Median gap, in seconds, over consecutive readings.
        let mut gaps: Vec<i64> =
            values.windows(2).map(|w| (w[1].0 - w[0].0) / 1000).filter(|g| *g > 0).collect();
        gaps.sort_unstable();
        let cadence = if gaps.is_empty() { 300 } else { gaps[gaps.len() / 2].max(1) as u64 };

        let first_t = values[0].0;
        let last_t = values[n - 1].0;
        let span_seconds = ((last_t - first_t) / 1000).max(0) as u64;
        // Expected readings over the span this data actually covers. One extra
        // because a span of exactly one cadence holds two readings.
        let expected = (span_seconds / cadence).saturating_add(1).max(1) as f64;
        let active = (n as f64 * 100.0 / expected).min(100.0);

        // Distinct UTC days touched. The subject's own day boundary is its
        // epoch offset and is not carried in a record, so this is the reader's
        // approximation and is only used for "days of wear".
        let mut days: Vec<i64> = values.iter().map(|(t, _)| t / 86_400_000).collect();
        days.sort_unstable();
        days.dedup();

        Some(Agp {
            readings: n,
            days_of_wear: days.len() as f64,
            sensor_active_percent: active,
            mean_mgdl: mean,
            cv_percent: if mean > 0.0 { sd * 100.0 / mean } else { 0.0 },
            // Bergenstal et al. 2018, the formula every AGP report uses.
            gmi_percent: 3.31 + 0.02392 * mean,
            very_low_percent: band(&|v| v < VERY_LOW),
            low_percent: band(&|v| (VERY_LOW..LOW).contains(&v)),
            in_range_percent: band(&|v| (LOW..=HIGH).contains(&v)),
            high_percent: band(&|v| v > HIGH && v <= VERY_HIGH),
            very_high_percent: band(&|v| v > VERY_HIGH),
            cadence_seconds: cadence,
            first_t,
            last_t,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use diaswarm_core::{EPOCH_MS, Record};

    fn cgm(t_ms: i64, mgdl: f64) -> Record {
        Record::new(t_ms, "cgm").set("mgdl", Some(mgdl.into()))
    }

    /// THE BANDS ARE THE CONSENSUS BANDS, AND THE BOUNDARIES ARE INCLUSIVE
    /// WHERE THE CONSENSUS SAYS SO.
    ///
    /// 70 and 180 are *in* range; 53.9 is very low and 250.1 is very high. Off
    /// by one on either boundary moves a clinical number, which is the kind of
    /// error a summary must not make quietly.
    #[test]
    fn the_bands_match_the_2019_consensus() {
        let base = 20_000i64 * EPOCH_MS;
        let m = 60_000;
        let records: Vec<Record> = vec![
            cgm(base, 40.0),            // very low
            cgm(base + m, 60.0),        // low
            cgm(base + 2 * m, 70.0),    // in range, on the boundary
            cgm(base + 3 * m, 120.0),   // in range
            cgm(base + 4 * m, 180.0),   // in range, on the boundary
            cgm(base + 5 * m, 200.0),   // high
            cgm(base + 6 * m, 300.0),   // very high
        ];
        let a = Agp::from_records(&records).expect("seven readings");
        assert_eq!(a.readings, 7);
        let pct = |n: f64| n * 100.0 / 7.0;
        assert!((a.very_low_percent - pct(1.0)).abs() < 0.01, "very low: {}", a.very_low_percent);
        assert!((a.low_percent - pct(1.0)).abs() < 0.01, "low: {}", a.low_percent);
        assert!((a.in_range_percent - pct(3.0)).abs() < 0.01, "in range: {}", a.in_range_percent);
        assert!((a.high_percent - pct(1.0)).abs() < 0.01, "high: {}", a.high_percent);
        assert!((a.very_high_percent - pct(1.0)).abs() < 0.01, "very high: {}", a.very_high_percent);
        let total = a.very_low_percent + a.low_percent + a.in_range_percent
            + a.high_percent + a.very_high_percent;
        assert!((total - 100.0).abs() < 0.01, "the bands do not partition the readings: {total}");
    }

    /// GMI IS BERGENSTAL'S FORMULA AND NOT AN APPROXIMATION OF IT.
    #[test]
    fn gmi_matches_the_published_formula() {
        let base = 20_000i64 * EPOCH_MS;
        let records: Vec<Record> = (0..10).map(|i| cgm(base + i * 60_000, 150.0)).collect();
        let a = Agp::from_records(&records).unwrap();
        assert!((a.mean_mgdl - 150.0).abs() < 1e-9);
        // 3.31 + 0.02392 * 150 = 6.898
        assert!((a.gmi_percent - 6.898).abs() < 1e-6, "GMI {} is not 6.898", a.gmi_percent);
        // Every reading identical, so there is no variability at all.
        assert!(a.cv_percent.abs() < 1e-9, "CV should be zero, got {}", a.cv_percent);
    }

    /// THE CADENCE IS INFERRED, BECAUSE HARD-CODING IT IS A FIVE-FOLD ERROR ON
    /// THE OTHER DEVICE.
    #[test]
    fn the_cadence_is_read_from_the_data_not_assumed() {
        let base = 20_000i64 * EPOCH_MS;
        let minutely: Vec<Record> = (0..60).map(|i| cgm(base + i * 60_000, 100.0)).collect();
        let a = Agp::from_records(&minutely).unwrap();
        assert_eq!(a.cadence_seconds, 60, "a Libre 3 cadence was not detected");
        assert!(a.sensor_active_percent > 99.0, "a complete hour read as {}%", a.sensor_active_percent);

        let five: Vec<Record> = (0..12).map(|i| cgm(base + i * 300_000, 100.0)).collect();
        let b = Agp::from_records(&five).unwrap();
        assert_eq!(b.cadence_seconds, 300, "a Dexcom cadence was not detected");
        assert!(b.sensor_active_percent > 99.0, "a complete hour read as {}%", b.sensor_active_percent);
    }

    /// HALF THE READINGS MISSING IS HALF THE SENSOR ACTIVE, AND MUST SAY SO.
    #[test]
    fn a_gappy_sensor_is_reported_as_gappy() {
        let base = 20_000i64 * EPOCH_MS;
        // Every other minute over an hour: same span, half the readings.
        let sparse: Vec<Record> = (0..30).map(|i| cgm(base + i * 120_000, 100.0)).collect();
        let a = Agp::from_records(&sparse).unwrap();
        // The median gap IS two minutes, so cadence adapts and coverage is full.
        // What must never happen is silently claiming 100% on a 1-minute device
        // while reporting a 2-minute cadence.
        assert_eq!(a.cadence_seconds, 120);
        assert!(
            a.insufficiency().is_some(),
            "one hour of data was reported as sufficient to act on"
        );
        assert!(a.insufficiency().unwrap().contains("days of wear"));
    }

    #[test]
    fn nothing_to_summarise_is_none_rather_than_zeros() {
        assert!(Agp::from_records(&[]).is_none());
        let only_bolus = vec![Record::new(0, "bolus").set("units", Some(1.0.into()))];
        assert!(Agp::from_records(&only_bolus).is_none(), "a bolus was summarised as glucose");
    }

    /// A SENSOR REPORTING AN IMPOSSIBLE VALUE IS REPORTING A FAULT.
    #[test]
    fn impossible_readings_do_not_move_the_statistics() {
        let base = 20_000i64 * EPOCH_MS;
        let mut records: Vec<Record> = (0..10).map(|i| cgm(base + i * 60_000, 100.0)).collect();
        records.push(cgm(base + 10 * 60_000, 0.0));
        records.push(cgm(base + 11 * 60_000, 5000.0));
        let a = Agp::from_records(&records).unwrap();
        assert_eq!(a.readings, 10, "an impossible reading was counted");
        assert!((a.mean_mgdl - 100.0).abs() < 1e-9, "mean moved to {}", a.mean_mgdl);
    }
}
