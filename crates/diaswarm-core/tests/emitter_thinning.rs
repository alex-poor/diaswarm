//! Does the emitter pass every reading through?
//!
//! It used to thin CGM to one reading per five minutes — spec §3.3 — and these
//! tests asserted that. The rule is gone: a streaming emitter can only remember
//! which buckets it has published as a high-water mark, a drain walks rows by
//! id rather than by time, and one row out of order made it discard every
//! earlier reading that followed. Measured on real history, 14,620 readings
//! lost.
//!
//! So the tests are inverted. What matters now is that nothing is dropped —
//! the failure this file exists to catch is a reading the loop saw and a
//! follower never will.

use diaswarm_core::{CGM_BUCKET_MS, Emitted, Record};

fn cgm(t: i64, mgdl: f64) -> Record {
    Record::new(t, "cgm").set("mgdl", Some(mgdl.into()))
}

/// A one-minute sensor reaches a follower as a one-minute sensor.
#[test]
fn every_reading_of_a_one_minute_sensor_is_published() {
    let mut e = Emitted::new();
    let base = 1_788_000_000_000i64;
    // Thirty minutes of Libre 3: a reading every 59 seconds, six to a bucket.
    let kept = (0..30).filter(|i| e.accept(&cgm(base + i * 59_000, 120.0 + *i as f64))).count();

    assert_eq!(kept, 30, "{} of 30 readings were dropped", 30 - kept);
}

/// Including across the pass boundaries that broke the old rule.
///
/// A bounded drain splits history into passes, each with a fresh emitter. The
/// thinning mark was carried between them, and that is precisely where it lost
/// data: a pass would treat everything up to the previous pass's newest bucket
/// as already published. Nothing is carried now, so nothing can be lost.
#[test]
fn a_new_pass_drops_nothing_it_inherited() {
    let base = 1_788_000_000_000i64;

    let mut first = Emitted::new();
    assert!(first.accept(&cgm(base + 10 * CGM_BUCKET_MS, 120.0)), "the first reading was refused");

    // A later pass, handed rows from EARLIER in the history — which is what a
    // drain walking by row id does.
    let mut second = Emitted::resuming(first.last_cgm_bucket());
    for i in 0..10 {
        assert!(
            second.accept(&cgm(base + i * CGM_BUCKET_MS, 130.0 + i as f64)),
            "a reading from an earlier bucket was dropped by a later pass"
        );
    }
}

/// An identical record delivered twice is still one record.
///
/// The sync queue resolves every version row to its current record, so the same
/// canonical bytes arrive many times — 22,003 for CGM on one real database.
/// Dropping those is deduplication, not thinning, and it stays.
#[test]
fn a_re_delivered_record_is_still_dropped() {
    let mut e = Emitted::new();
    let r = cgm(1_788_000_000_000, 120.0);
    assert!(e.accept(&r));
    assert!(!e.accept(&r), "the same record was published twice");
    assert!(!e.accept(&r));
}
