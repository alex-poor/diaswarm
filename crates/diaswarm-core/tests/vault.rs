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
fn truncating_the_tail_is_not_detectable_from_one_copy_alone() {
    // A hash chain catches modification and reordering. It does NOT catch the
    // owner deleting the END of their own log: what remains is a valid prefix,
    // and the subject holds every key needed to re-sign it.
    //
    // REPLICATION IS WHAT CLOSES THIS, and it is not a new mechanism — it is the
    // swarm. Once peers hold the log, truncating this copy is equivocation
    // rather than deletion, and the disagreement between copies is the evidence.
    // §7.4 lists "publication is permanent" as a cost; for the grant log it is
    // the benefit.
    //
    // So this test pins what ONE COPY can establish, which is the state today.
    // If it ever starts failing, something has begun tracking the head and the
    // reasoning above needs revisiting rather than the test being deleted.
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

#[test]
fn sealing_a_segment_again_adds_to_it_rather_than_replacing_it() {
    // THE BUG THIS EXISTS FOR. A caller passes what it has just collected, not
    // the whole day. Replacing the segment with that destroyed everything
    // sealed earlier — silently, from the only copy the subject had. It showed
    // up as a reader's record count FALLING between two fetches.
    let dir = tempdir::TempDir::new("append").unwrap();
    let subject = Identity::generate();
    let reader = Identity::generate();
    let vault = Vault::create(dir.path(), &subject, OFFSET).unwrap();
    let epoch = 20_000;

    vault.record_grant(&subject, &reader.enc_public(), "follow", "grant", 0).unwrap();

    // Three passes, each carrying only what arrived since the last.
    vault.seal(epoch, &day(epoch, 100.0)).unwrap();
    vault.seal(epoch, &day(epoch + 0, 101.0)).unwrap();
    vault.seal(epoch, &day(epoch + 0, 102.0)).unwrap();
    vault.publish_wraps(&subject, &reader.enc_public(), "follow").unwrap();

    let opened = vault.read_as(&reader, "follow").unwrap();
    let values: Vec<f64> = opened[&epoch]
        .iter()
        .filter_map(|r| r.get("mgdl").and_then(|v| v.as_f64()))
        .collect();
    assert_eq!(values, vec![100.0, 101.0, 102.0], "a re-seal dropped earlier records");
}

#[test]
fn resealing_the_same_records_does_not_duplicate_them() {
    // A full resync re-drains everything. Without the check that would append
    // the whole history to a segment that already had it.
    let dir = tempdir::TempDir::new("dedupe").unwrap();
    let subject = Identity::generate();
    let reader = Identity::generate();
    let vault = Vault::create(dir.path(), &subject, OFFSET).unwrap();
    let epoch = 20_000;

    vault.record_grant(&subject, &reader.enc_public(), "follow", "grant", 0).unwrap();
    vault.seal(epoch, &day(epoch, 100.0)).unwrap();
    vault.seal(epoch, &day(epoch, 100.0)).unwrap();
    vault.seal(epoch, &day(epoch, 100.0)).unwrap();
    vault.publish_wraps(&subject, &reader.enc_public(), "follow").unwrap();

    assert_eq!(
        vault.read_as(&reader, "follow").unwrap()[&epoch].len(),
        1,
        "the same record was sealed three times and appeared more than once"
    );
}

#[test]
fn a_reader_does_not_see_a_record_twice_when_segments_overlap() {
    // A rotation starts a new segment for the same epoch, and a resync writes
    // records into it that an earlier segment already held. Neither is a fault.
    // A reader that trusted segments to be disjoint would double-count insulin.
    let dir = tempdir::TempDir::new("overlap").unwrap();
    let subject = Identity::generate();
    let reader = Identity::generate();
    let vault = Vault::create(dir.path(), &subject, OFFSET).unwrap();
    let epoch = 20_000;

    vault.record_grant(&subject, &reader.enc_public(), "follow", "grant", 0).unwrap();
    vault.seal(epoch, &day(epoch, 100.0)).unwrap();
    vault.rotate().unwrap();                       // as a revocation would
    vault.seal(epoch, &day(epoch, 100.0)).unwrap(); // the same record again
    vault.seal(epoch, &day(epoch, 101.0)).unwrap();
    vault.publish_wraps(&subject, &reader.enc_public(), "follow").unwrap();

    assert!(vault.segments().unwrap().len() >= 2, "the rotation did not split");
    let opened = vault.read_as(&reader, "follow").unwrap();
    let values: Vec<f64> = opened[&epoch]
        .iter()
        .filter_map(|r| r.get("mgdl").and_then(|v| v.as_f64()))
        .collect();
    assert_eq!(values, vec![100.0, 101.0], "overlapping segments were double-counted");
}

/// A GRANT MUST KEEP WORKING TOMORROW.
///
/// The bug this pins: wrapping happened only when a grant was made, so a
/// reader received the segments that existed at that instant and nothing
/// after. Sealing carries on cutting a segment per epoch, and every one of
/// them was unopenable — the reader kept syncing, kept receiving bytes, and
/// kept reporting the same stale reading with no error anywhere. Seen in the
/// field as 123 segments fetched and 0 wraps.
///
/// Every earlier test granted after all the sealing was done, which made the
/// grant-time wrap sufficient by construction. That ordering is the whole
/// test: grant FIRST, seal AFTER.
#[test]
fn a_grant_covers_segments_sealed_after_it() {
    let dir = tempdir::TempDir::new("later").unwrap();
    let subject = Identity::generate();
    let reader = Identity::generate();
    let vault = Vault::create(dir.path(), &subject, OFFSET).unwrap();

    // Day one exists, and the reader is granted while that is all there is.
    let base: i64 = 20_100;
    vault.seal(base, &day(base, 5.0)).unwrap();
    vault.record_grant(&subject, &reader.enc_public(), "follow", "grant", 0).unwrap();
    vault.publish_wraps(&subject, &reader.enc_public(), "follow").unwrap();

    // Then the days that made the bug: nothing further is granted, because in
    // real use nothing further is. The subject simply keeps looping.
    for i in 1..5 {
        vault.seal(base + i, &day(base + i, 5.0 + i as f64)).unwrap();
    }

    let read = vault.read_as(&reader, "follow").unwrap();
    let epochs: Vec<i64> = read.keys().copied().collect();
    assert_eq!(
        epochs,
        (0..5).map(|i| base + i).collect::<Vec<_>>(),
        "a reader granted on day one must be able to open days two onward; \
         got only {epochs:?}"
    );
}

/// The same thing through a rotation rather than an epoch boundary.
///
/// Revoking one reader rotates the vault, which cuts a fresh segment inside
/// the SAME epoch. Everyone still granted has to be wrapped for it, or a
/// reader loses the rest of the day whenever some unrelated reader is
/// withdrawn — a revocation that silently revokes more than it names.
#[test]
fn rotating_for_one_reader_does_not_cut_off_another() {
    let dir = tempdir::TempDir::new("rot").unwrap();
    let subject = Identity::generate();
    let staying = Identity::generate();
    let leaving = Identity::generate();
    let vault = Vault::create(dir.path(), &subject, OFFSET).unwrap();

    let epoch: i64 = 20_200;
    vault.record_grant(&subject, &staying.enc_public(), "follow", "grant", 0).unwrap();
    vault.record_grant(&subject, &leaving.enc_public(), "follow", "grant", 0).unwrap();
    vault.seal(epoch, &day(epoch, 5.0)).unwrap();

    vault.revoke(&subject, &leaving.enc_public(), "follow").unwrap();

    // Later in the same day, into the segment the rotation cut.
    let later = vec![
        Record::new(epoch * EPOCH_MS + 7_200_000, "cgm").set("mgdl", Some(9.0.into()))
    ];
    vault.seal(epoch, &later).unwrap();

    let stayed = vault.read_as(&staying, "follow").unwrap();
    assert_eq!(
        stayed.get(&epoch).map(|r| r.len()),
        Some(2),
        "a reader who was not revoked must still get the rest of the day"
    );
    let left = vault.read_as(&leaving, "follow").unwrap();
    assert_eq!(
        left.get(&epoch).map(|r| r.len()),
        Some(1),
        "the revoked reader keeps what they had and gets nothing after"
    );
}

/// Re-stating a grant that already stands must not grow the log.
///
/// It has to stay re-runnable, because re-granting is how a subject repairs a
/// vault whose wraps went missing; a hash-chained log that gained a line every
/// time someone toggled a setting would make the real history unreadable.
#[test]
fn re_granting_is_idempotent_but_still_repairs() {
    let dir = tempdir::TempDir::new("idem").unwrap();
    let subject = Identity::generate();
    let reader = Identity::generate();
    let vault = Vault::create(dir.path(), &subject, OFFSET).unwrap();

    let epoch: i64 = 20_300;
    vault.seal(epoch, &day(epoch, 5.0)).unwrap();
    vault.record_grant(&subject, &reader.enc_public(), "follow", "grant", 0).unwrap();
    vault.publish_wraps(&subject, &reader.enc_public(), "follow").unwrap();
    assert_eq!(vault.grants().unwrap().len(), 1);

    // Delete the wraps, the way the field failure left them, and re-grant.
    std::fs::remove_dir_all(dir.path().join("wraps")).unwrap();
    assert!(vault.read_as(&reader, "follow").unwrap().is_empty());

    vault.record_grant(&subject, &reader.enc_public(), "follow", "grant", 0).unwrap();
    let repaired = vault.publish_wraps(&subject, &reader.enc_public(), "follow").unwrap();
    assert_eq!(repaired, 1, "re-granting must re-issue the missing wrap");
    assert_eq!(vault.grants().unwrap().len(), 1, "and must not append to the log");
    assert!(!vault.read_as(&reader, "follow").unwrap().is_empty());
}

/// The subject's book of readers must not be something a peer can ask for.
///
/// It is the mapping the grant log deliberately does not contain (D13). A
/// vault is copied wholesale by design, so the check that matters is that the
/// book is not part of what a copy carries.
#[test]
fn the_reader_book_is_not_served() {
    let dir = tempdir::TempDir::new("book").unwrap();
    let subject = Identity::generate();
    let reader = Identity::generate();
    let vault = Vault::create(dir.path(), &subject, OFFSET).unwrap();
    vault.record_grant(&subject, &reader.enc_public(), "follow", "grant", 0).unwrap();

    let book = dir.path().join("readers.json");
    assert!(book.exists(), "the subject must remember who they granted to");
    let text = std::fs::read_to_string(&book).unwrap();
    assert!(
        text.contains(&diaswarm_core::vault::hex(&reader.enc_public())),
        "the book has to hold the actual key, or it cannot wrap"
    );

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&book).unwrap().permissions().mode();
        assert_eq!(mode & 0o077, 0, "the book must not be group- or world-readable");
    }
}
