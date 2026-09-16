//! A cached read must never be a stale read.
//!
//! **THE FAILURE THIS GUARDS AGAINST HAS NO EXCEPTION AND NO LOG LINE.**
//! `read_as_from` caches its result against a fingerprint of the vault on
//! disk — one `stat` per segment and per wrap — because a profile of the
//! follower on 2026-09-17 put `netProfile` and `netTempTarget` at 27% of all
//! cycles between them, each decrypting the whole grant on every refresh.
//!
//! If that fingerprint misses a change, the reader returns the previous
//! answer: a follower showing glucose from before the last reading, with
//! nothing raised anywhere. That is worse than the cost it replaces, so every
//! way the vault can change under a reader gets a case here.
//!
//! The vault's own history is the reason for the paranoia — "found by watching
//! a reader's record count fall from 32,150 to 27,874 between two fetches".

use diaswarm_core::vault::{Identity, Vault};
use diaswarm_core::{EPOCH_MS, Record};

const OFFSET: i64 = 12 * 3_600_000;

fn tmp(tag: &str) -> std::path::PathBuf {
    let p = std::env::temp_dir().join(format!(
        "diaswarm-readcache-{tag}-{}",
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
    ));
    std::fs::create_dir_all(&p).unwrap();
    p
}

fn rec(epoch: i64, at: i64, mgdl: f64) -> Record {
    Record::new(epoch * EPOCH_MS + at, "cgm").set("mgdl", Some(mgdl.into()))
}

fn count(v: &Vault, reader: &Identity) -> usize {
    v.read_as(reader, "follow").expect("read").values().map(Vec::len).sum()
}

/// A second reading sealed into the SAME epoch must appear. This is the case a
/// length-blind fingerprint would miss, and it is the common one: a subject
/// flushes into today's segment every minute.
#[test]
fn a_record_appended_to_todays_segment_is_seen_by_the_next_read() {
    let root = tmp("append");
    let subject = Identity::generate();
    let reader = Identity::generate();
    let vault = Vault::create(&root, &subject, OFFSET).expect("create");
    vault.seal(20_000, &[rec(20_000, 3_600_000, 100.0)]).expect("seal");
    let reader_pub: [u8; 32] = x25519_dalek::PublicKey::from(&reader.encryption).to_bytes();
    vault.record_grant(&subject, &reader_pub, "follow", "grant", 0).expect("grant");
    vault.publish_wraps(&subject, &reader_pub, "follow").expect("wraps");

    let first = count(&vault, &reader);
    assert_eq!(first, 1, "the first read should see the one sealed record");

    // Warm the cache, then append to the same epoch.
    assert_eq!(count(&vault, &reader), 1, "a repeat read must agree with itself");
    vault.seal(20_000, &[rec(20_000, 7_200_000, 111.0)]).expect("seal again");
    vault.publish_wraps(&subject, &reader_pub, "follow").expect("wraps again");

    assert_eq!(
        count(&vault, &reader),
        2,
        "a record appended to an epoch already read must appear — a cache that \
         misses this shows a follower the reading before last, with no error"
    );
}

/// A whole new epoch must appear. This is the easy case — a new file — but it
/// is the one a fingerprint built only from the newest segment would still get
/// wrong.
#[test]
fn a_new_epoch_is_seen_by_the_next_read() {
    let root = tmp("newepoch");
    let subject = Identity::generate();
    let reader = Identity::generate();
    let vault = Vault::create(&root, &subject, OFFSET).expect("create");
    vault.seal(21_000, &[rec(21_000, 3_600_000, 100.0)]).expect("seal");
    let reader_pub: [u8; 32] = x25519_dalek::PublicKey::from(&reader.encryption).to_bytes();
    vault.record_grant(&subject, &reader_pub, "follow", "grant", 0).expect("grant");
    vault.publish_wraps(&subject, &reader_pub, "follow").expect("wraps");
    assert_eq!(count(&vault, &reader), 1);

    vault.seal(21_001, &[rec(21_001, 3_600_000, 120.0)]).expect("seal next day");
    vault.publish_wraps(&subject, &reader_pub, "follow").expect("wraps");
    assert_eq!(count(&vault, &reader), 2, "tomorrow must appear");
}

/// A wrap granted after a read must open the segment it was granted for. The
/// segments do not change here at all — only the wraps do — so a fingerprint
/// that watched segments alone would serve the pre-grant answer for ever.
#[test]
fn a_grant_made_after_a_read_takes_effect() {
    let root = tmp("lategrant");
    let subject = Identity::generate();
    let reader = Identity::generate();
    let vault = Vault::create(&root, &subject, OFFSET).expect("create");
    vault.seal(22_000, &[rec(22_000, 3_600_000, 100.0)]).expect("seal");

    // Read BEFORE the grant: nothing is openable, and that answer gets cached.
    assert_eq!(count(&vault, &reader), 0, "an ungranted reader opens nothing");

    let reader_pub: [u8; 32] = x25519_dalek::PublicKey::from(&reader.encryption).to_bytes();
    vault.record_grant(&subject, &reader_pub, "follow", "grant", 0).expect("grant");
    vault.publish_wraps(&subject, &reader_pub, "follow").expect("wraps");

    assert_eq!(
        count(&vault, &reader),
        1,
        "the grant must take effect on the next read — only the wraps changed, \
         so a segments-only fingerprint would still say zero"
    );
}

/// Two readers must not receive each other's answers. The tag is part of the
/// key; if it were not, whichever read first would answer for both.
#[test]
fn two_readers_do_not_share_a_cached_answer() {
    let root = tmp("tworeaders");
    let subject = Identity::generate();
    let granted = Identity::generate();
    let stranger = Identity::generate();
    let vault = Vault::create(&root, &subject, OFFSET).expect("create");
    vault.seal(23_000, &[rec(23_000, 3_600_000, 100.0)]).expect("seal");
    let pubkey: [u8; 32] = x25519_dalek::PublicKey::from(&granted.encryption).to_bytes();
    vault.record_grant(&subject, &pubkey, "follow", "grant", 0).expect("grant");
    vault.publish_wraps(&subject, &pubkey, "follow").expect("wraps");

    assert_eq!(count(&vault, &granted), 1, "the granted reader sees the record");
    assert_eq!(
        count(&vault, &stranger),
        0,
        "a stranger must see nothing even though a granted read was just cached"
    );
    assert_eq!(count(&vault, &granted), 1, "and the granted reader still sees it");
}
