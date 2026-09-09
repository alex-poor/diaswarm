//! The property, through the on-disk form a phone would actually write.
//!
//! `seal_interop.rs` proves the construction agrees across languages. This
//! proves the vault built on it keeps the promise — sealing a day at a time,
//! with the revocation landing while there is still history to come. Sealing
//! everything first and revoking afterwards would make "nothing new" true by
//! construction rather than by mechanism, which is how an earlier version of
//! this test passed without testing anything.

use diaswarm_core::vault::{Identity, Vault};
use diaswarm_core::{Record, EPOCH_MS};

/// UTC+12: the offset that made this whole redesign necessary.
const OFFSET: i64 = 12 * 3_600_000;

fn day(epoch: i64, marker: f64) -> Vec<Record> {
    vec![Record::new(epoch * EPOCH_MS + 3_600_000, "cgm").set("mgdl", Some(marker.into()))]
}

struct Run {
    dir: tempdir::TempDir,
    subject: Identity,
    a: Identity,
    b: Identity,
}

mod tempdir {
    pub struct TempDir(std::path::PathBuf);
    impl TempDir {
        pub fn new(tag: &str) -> std::io::Result<Self> {
            let p = std::env::temp_dir().join(format!(
                "diaswarm-{tag}-{}",
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            std::fs::create_dir_all(&p)?;
            Ok(TempDir(p))
        }
        pub fn path(&self) -> &std::path::Path {
            &self.0
        }
    }
}

fn live_run(days: i64, revoke_after: i64) -> (Run, Vault, std::collections::BTreeSet<i64>) {
    let dir = tempdir::TempDir::new("vault").unwrap();
    let subject = Identity::generate();
    let a = Identity::generate();
    let b = Identity::generate();
    let vault = Vault::create(dir.path(), &subject, OFFSET).unwrap();

    let base: i64 = 20_000;
    vault.record_grant(&subject, &a.enc_public(), "follow", "grant", 0).unwrap();
    vault.record_grant(&subject, &b.enc_public(), "follow", "grant", 0).unwrap();

    let mut held_at_stop = std::collections::BTreeSet::new();
    for i in 0..days {
        let epoch = base + i;
        if i == revoke_after {
            held_at_stop = vault.read_as(&a, "follow").unwrap().keys().copied().collect();
            vault.revoke(&subject, &a.enc_public(), "follow").unwrap();
        }
        vault.seal(epoch, &day(epoch, 100.0 + i as f64)).unwrap();
        vault.publish_wraps(&subject, &a.enc_public(), "follow").unwrap();
        vault.publish_wraps(&subject, &b.enc_public(), "follow").unwrap();
    }
    (Run { dir, subject, a, b }, vault, held_at_stop)
}

#[test]
fn a_revoked_reader_gains_nothing_and_keeps_everything() {
    let (run, vault, held_at_stop) = live_run(10, 6);
    let after: std::collections::BTreeSet<i64> =
        vault.read_as(&run.a, "follow").unwrap().keys().copied().collect();

    assert!(after.difference(&held_at_stop).next().is_none(), "gained an epoch after the stop");
    assert!(held_at_stop.is_subset(&after), "lost an epoch it already held");
    assert_eq!(after.len(), 6, "revocation did not bite: holds {} of 10", after.len());
    assert_eq!(vault.read_as(&run.b, "follow").unwrap().len(), 10, "the other reader was affected");
    let _ = run.dir.path();
}

#[test]
fn revocation_cuts_mid_day_not_at_the_next_boundary() {
    // THE REASON SEGMENTS EXIST. Waiting for the epoch boundary meant that at
    // UTC+12, revoking at 9pm left a reader everything until noon the next day.
    let dir = tempdir::TempDir::new("midday").unwrap();
    let subject = Identity::generate();
    let reader = Identity::generate();
    let vault = Vault::create(dir.path(), &subject, OFFSET).unwrap();
    let epoch = 20_000;

    vault.record_grant(&subject, &reader.enc_public(), "follow", "grant", 0).unwrap();

    // The morning of one day.
    vault.seal(epoch, &day(epoch, 100.0)).unwrap();
    vault.publish_wraps(&subject, &reader.enc_public(), "follow").unwrap();
    assert_eq!(vault.read_as(&reader, "follow").unwrap()[&epoch].len(), 1);

    // Revoked at lunchtime — same epoch, hours before the boundary.
    vault.revoke(&subject, &reader.enc_public(), "follow").unwrap();

    // The afternoon of the SAME day.
    vault.seal(epoch, &day(epoch, 200.0)).unwrap();
    vault.publish_wraps(&subject, &reader.enc_public(), "follow").unwrap();

    let opened = vault.read_as(&reader, "follow").unwrap();
    let values: Vec<f64> = opened[&epoch]
        .iter()
        .filter_map(|r| r.get("mgdl").and_then(|v| v.as_f64()))
        .collect();
    assert_eq!(
        values,
        vec![100.0],
        "the reader saw the afternoon of a day they were revoked from at lunchtime"
    );
    assert!(vault.segments().unwrap().len() >= 2, "the day was not split into segments");
}

#[test]
fn what_a_revoked_reader_keeps_still_decrypts_correctly() {
    let (run, vault, _) = live_run(10, 6);
    let opened = vault.read_as(&run.a, "follow").unwrap();
    for (i, (_, records)) in opened.iter().enumerate() {
        assert_eq!(
            records[0].get("mgdl").and_then(|v| v.as_f64()),
            Some(100.0 + i as f64),
            "an epoch opened but yielded the wrong records"
        );
    }
    let _ = run.subject.enc_public();
}

#[test]
fn a_stranger_holding_the_whole_vault_reads_nothing() {
    let (_run, vault, _) = live_run(4, 3);
    let stranger = Identity::generate();
    assert!(vault.read_as(&stranger, "follow").unwrap().is_empty(), "a stranger opened something");
}

#[test]
fn grants_are_signed_and_tamper_evident() {
    let (run, vault, _) = live_run(3, 2);
    let grants = vault.grants().unwrap();
    assert!(grants.iter().any(|g| g.act == "stop"), "the withdrawal was not recorded");
    for g in &grants {
        assert!(Vault::verify(g, &run.subject.verifying()), "a grant did not verify");
    }
    let mut edited = grants[0].clone();
    edited.segment += 1;
    assert!(!Vault::verify(&edited, &run.subject.verifying()), "an edited grant verified");

    let impostor = Identity::generate();
    assert!(!Vault::verify(&grants[0], &impostor.verifying()), "verified against a foreign key");
}

#[test]
fn a_vault_from_another_spec_version_is_refused_not_guessed_at() {
    let dir = tempdir::TempDir::new("spec").unwrap();
    let subject = Identity::generate();
    Vault::create(dir.path(), &subject, OFFSET).unwrap();
    let meta = dir.path().join("meta.json");
    let text = std::fs::read_to_string(&meta).unwrap().replace("\"spec\": 3", "\"spec\": 99");
    std::fs::write(&meta, text).unwrap();
    assert!(
        matches!(Vault::open(dir.path()), Err(diaswarm_core::vault::VaultError::SpecMismatch { .. })),
        "a vault written by an unknown spec version must be refused"
    );
}

#[test]
fn the_offset_is_recorded_and_read_back() {
    // A vault re-cut at a different phase is a different set of days, so the
    // offset must survive a round trip rather than being re-guessed on open.
    let dir = tempdir::TempDir::new("offset").unwrap();
    let subject = Identity::generate();
    Vault::create(dir.path(), &subject, OFFSET).unwrap();
    assert_eq!(Vault::open(dir.path()).unwrap().offset(), OFFSET);
}

#[test]
fn resealing_a_segment_does_not_invalidate_wraps_already_published() {
    let dir = tempdir::TempDir::new("reseal").unwrap();
    let subject = Identity::generate();
    let reader = Identity::generate();
    let vault = Vault::create(dir.path(), &subject, OFFSET).unwrap();
    let epoch = 20_000;

    vault.record_grant(&subject, &reader.enc_public(), "follow", "grant", 0).unwrap();
    vault.seal(epoch, &day(epoch, 100.0)).unwrap();
    vault.publish_wraps(&subject, &reader.enc_public(), "follow").unwrap();
    assert_eq!(vault.read_as(&reader, "follow").unwrap().len(), 1);

    let mut fuller = day(epoch, 100.0);
    fuller.extend(day(epoch, 101.0));
    vault.seal(epoch, &fuller).unwrap();

    let opened = vault.read_as(&reader, "follow").unwrap();
    assert_eq!(opened.len(), 1, "the reader's wrap stopped opening the segment");
    assert_eq!(opened[&epoch].len(), 2, "the reseal did not include the newer records");
}

#[test]
fn the_grant_log_names_nobody() {
    // §12.0: the data was encrypted and the social graph was not.
    let (run, vault, _) = live_run(3, 2);
    let raw = std::fs::read_to_string(run.dir.path().join("grants.ndjson")).unwrap();
    let reader_hex = diaswarm_core::vault::hex(&run.a.enc_public());

    assert!(!raw.contains(&reader_hex), "the log still contains a reader's public key");
    assert!(!raw.contains("follow"), "the log still names the purpose");

    // And the wrap filenames, which leaked exactly as much.
    let wraps = std::fs::read_dir(run.dir.path().join("wraps")).unwrap();
    for seg in wraps.flatten() {
        for f in std::fs::read_dir(seg.path()).unwrap().flatten() {
            let name = f.file_name().to_string_lossy().to_string();
            assert!(!name.contains(&reader_hex), "a wrap filename is a reader's key");
        }
    }
}

#[test]
fn the_same_reader_is_a_different_tag_to_a_different_subject() {
    // The leak that mattered most: one clinician granted by many people used to
    // appear as the SAME key in every log, identifying them and clustering
    // their patients.
    use diaswarm_core::seal::grant_tag;
    let alice = Identity::generate();
    let bob = Identity::generate();
    let clinician = Identity::generate();

    let from_alice = grant_tag(&alice.encryption, &clinician.enc_public(), "clinician");
    let from_bob = grant_tag(&bob.encryption, &clinician.enc_public(), "clinician");
    assert_ne!(from_alice, from_bob, "the same reader is linkable across subjects");

    // The reader still finds their own, from their side of the shared secret.
    assert_eq!(
        from_alice,
        grant_tag(&clinician.encryption, &alice.enc_public(), "clinician"),
        "the reader cannot compute the tag they were filed under"
    );
    // And a different purpose is a different tag, so the word is never needed.
    assert_ne!(
        from_alice,
        grant_tag(&alice.encryption, &clinician.enc_public(), "cohort"),
        "purposes share a tag"
    );
}

#[test]
fn altering_or_reordering_the_log_is_detectable() {
    let (run, vault, _) = live_run(3, 2);
    assert_eq!(vault.verify_chain(&run.subject.verifying()).unwrap(), None);

    // Remove an entry from the MIDDLE: the links no longer join up.
    let path = run.dir.path().join("grants.ndjson");
    let text = std::fs::read_to_string(&path).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    assert!(lines.len() >= 3, "need three entries to drop a middle one");
    let without_middle: Vec<&str> = lines
        .iter()
        .enumerate()
        .filter(|(i, _)| *i != 1)
        .map(|(_, l)| *l)
        .collect();
    std::fs::write(&path, without_middle.join("\n") + "\n").unwrap();

    assert!(
        vault.verify_chain(&run.subject.verifying()).unwrap().is_some(),
        "an entry was removed from the middle of the log and nothing noticed"
    );
}

#[test]
fn truncating_the_tail_is_not_detectable_here_and_that_is_the_honest_limit() {
    // A hash chain catches modification and reordering. It does NOT catch the
    // owner deleting the END of their own log, because what remains is a valid
    // prefix — and the subject holds every key needed to re-sign it anyway.
    //
    // This is feasibility.md §6 arriving in code: every mechanism reviewed
    // produces "an account the reader cannot quietly rewrite", and none
    // produces "an account they cannot simply omit". Detecting a dropped tail
    // needs someone else to have seen it — a reader who remembers, or a
    // transparency log that gossips. Neither exists yet.
    //
    // Asserted rather than left implicit, so nobody later reads "tamper-evident"
    // as more than it is.
    let (run, vault, _) = live_run(3, 2);
    let path = run.dir.path().join("grants.ndjson");
    let text = std::fs::read_to_string(&path).unwrap();
    let kept: Vec<&str> = text.lines().take(text.lines().count() - 1).collect();
    std::fs::write(&path, kept.join("\n") + "\n").unwrap();

    assert_eq!(
        vault.verify_chain(&run.subject.verifying()).unwrap(),
        None,
        "if this now fails, tail truncation became detectable and §6 needs revisiting"
    );
}
