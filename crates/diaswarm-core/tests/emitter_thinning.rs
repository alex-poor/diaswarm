//! Does the emitter thin CGM the way `spec/records.md` §3.3 says?
//!
//! This is the rule that was measured broken on real hardware. The on-device
//! emitter kept every reading while `tools/canon.py` dropped four in five, so
//! the two implementations disagreed by 3,898 records over 74 days — and
//! nothing noticed, because each was self-consistent.
//!
//! The subject's Libre 3 reports every 59 seconds. These tests use that shape
//! rather than a convenient one.

use diaswarm_core::{CGM_BUCKET_MS, Emitted, Record};

fn cgm(t: i64, mgdl: f64) -> Record {
    Record::new(t, "cgm").set("mgdl", Some(mgdl.into()))
}

/// A one-minute sensor should reach a follower as a five-minute one.
#[test]
fn a_one_minute_sensor_is_thinned_to_one_in_five() {
    let mut e = Emitted::new();
    let base = 1_788_000_000_000i64;
    // 30 minutes of Libre 3: a reading every 59 seconds.
    let kept: Vec<i64> =
        (0..30).map(|i| base + i * 59_000).filter(|t| e.accept(&cgm(*t, 120.0))).collect();

    assert_eq!(
        kept.len(),
        6,
        "30 minutes should reach a follower as 6 readings, not {}",
        kept.len()
    );
    // KEEPING THE FIRST, not the last and not a mean: the first is the reading
    // the loop actually saw and dosed on.
    assert_eq!(kept[0], base, "the first reading of the run was not the one kept");
    for pair in kept.windows(2) {
        assert!(
            pair[1].div_euclid(CGM_BUCKET_MS) > pair[0].div_euclid(CGM_BUCKET_MS),
            "two readings escaped from the same bucket"
        );
    }
}

/// Nothing else is thinned. A bolus a second after another is two boluses.
#[test]
fn only_cgm_is_thinned() {
    let mut e = Emitted::new();
    let base = 1_788_000_000_000i64;
    let mut kept = 0;
    for i in 0..10 {
        let r = Record::new(base + i * 1_000, "bolus").set("units", Some((1.0 + i as f64).into()));
        if e.accept(&r) {
            kept += 1;
        }
    }
    assert_eq!(kept, 10, "boluses were thinned as though they were readings");
}

/// THE PART THAT ONLY MATTERS LIVE.
///
/// Readings arrive one per minute, one pass at a time, so an emitter that
/// starts empty every pass never sees two readings from one bucket together
/// and thins nothing at all. The mark has to survive the run.
#[test]
fn thinning_survives_a_restart() {
    let base = 1_788_000_000_000i64;

    let mut first = Emitted::new();
    assert!(first.accept(&cgm(base, 120.0)));
    let carried = first.last_cgm_bucket();
    assert!(carried.is_some());

    // A new pass, a minute later, in the same bucket.
    let mut second = Emitted::resuming(carried);
    assert!(
        !second.accept(&cgm(base + 60_000, 121.0)),
        "a reading in an already-emitted bucket got through after a restart"
    );

    // And the next bucket is let through.
    let mut third = Emitted::resuming(second.last_cgm_bucket());
    assert!(
        third.accept(&cgm(base + CGM_BUCKET_MS, 122.0)),
        "the next bucket was refused, so the stream would stop"
    );
}

/// An emitter that was never told a mark must not silently drop history.
#[test]
fn a_fresh_emitter_keeps_the_first_reading_it_sees() {
    let mut e = Emitted::resuming(None);
    assert!(e.accept(&cgm(1_788_000_000_000, 120.0)));
    assert_eq!(e.thinned, 0);
}

/// OUT-OF-ORDER ROWS MUST NOT BE THINNED AWAY.
///
/// The sync queue walks rows by id, not by time, so a record can arrive with a
/// timestamp older than one already seen. A high-water mark alone drops it —
/// which is not thinning, it is losing a reading the loop acted on. Measured on
/// a real re-drain as roughly 1,100 readings.
#[test]
fn a_late_arriving_older_reading_survives() {
    let mut e = Emitted::new();
    let base = 1_788_000_000_000i64;

    // Three buckets, arriving newest first.
    assert!(e.accept(&cgm(base + 2 * CGM_BUCKET_MS, 120.0)), "the newest was refused");
    assert!(
        e.accept(&cgm(base + CGM_BUCKET_MS, 121.0)),
        "a reading from an earlier bucket was thinned away by a high-water mark"
    );
    assert!(
        e.accept(&cgm(base, 122.0)),
        "the earliest bucket was thinned away by a high-water mark"
    );
    assert_eq!(e.thinned, 0, "nothing shared a bucket, so nothing should have been thinned");
}

/// A record re-delivered by a version row is a duplicate, not a thinning.
///
/// The sync queue resolves every version row to the current record, so the same
/// canonical bytes arrive many times — 22,003 for CGM on one real database.
/// Counting those as thinned reported 27,036 where 3,898 were genuinely thinned.
#[test]
fn re_delivery_is_not_counted_as_thinning() {
    let mut e = Emitted::new();
    let r = cgm(1_788_000_000_000, 120.0);

    assert!(e.accept(&r));
    assert!(!e.accept(&r), "the same record was emitted twice");
    assert!(!e.accept(&r));
    assert_eq!(e.thinned, 0, "re-delivery was counted as thinning: {}", e.thinned);
}
