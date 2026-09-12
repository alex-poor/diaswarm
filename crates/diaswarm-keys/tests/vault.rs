//! Does D26 hold with production types and segments on disk?
//!
//! `spike/p2panda-datascheme` proved the properties in memory with the crate's
//! `test_utils` helpers. This asks the same questions of the real thing: our own
//! DGM and orderer, `p2panda_core::VerifyingKey` and `Hash` as the identifiers,
//! and segments written to and read from a directory.

use std::path::PathBuf;
use std::time::Instant;

use diaswarm_core::{EPOCH_MS, Record};
use diaswarm_keys::{Vault, wire};
use p2panda_core::SigningKey;
use p2panda_encryption::Rng;
use p2panda_store::{SqliteStore, SqliteStoreBuilder};

const OFFSET: i64 = 12 * 3_600_000;

fn tmp(tag: &str) -> PathBuf {
    let p = std::env::temp_dir().join(format!(
        "diaswarm-keys-{tag}-{}",
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
    ));
    std::fs::create_dir_all(&p).unwrap();
    p
}

fn day(epoch: i64) -> Vec<Record> {
    (0..288i64)
        .map(|i| {
            Record::new(epoch * EPOCH_MS + i * 300_000, "cgm")
                .set("mgdl", Some((100.0 + (i % 80) as f64).into()))
        })
        .collect()
}

/// THE WHOLE DECISION, END TO END.
///
/// A subject seals days into segments, a reader is granted and opens them, the
/// recent end stays cheap as history grows, and a revoked reader is cut off from
/// what comes next while keeping what it already had.
#[tokio::test(flavor = "multi_thread")]
async fn a_granted_reader_opens_segments_and_a_revoked_one_stops() {
    let rng = Rng::default();
    let subject_key = SigningKey::from_bytes(&rand32());
    let reader_key = SigningKey::from_bytes(&rand32());

    let root = tmp("shared");
    let store = SqliteStoreBuilder::memory().build().await.expect("store");
    let mut subject = Vault::open(&root, OFFSET, &subject_key).expect("subject vault");
    let (subject_mgr, subject_bundle) = Vault::key_bundle(&rng).expect("subject bundle");
    let (reader_mgr, reader_bundle) = Vault::key_bundle(&rng).expect("reader bundle");

    subject.create(subject_mgr).expect("create");

    // A day before anybody is granted.
    subject.seal(20_000, &day(20_000)).expect("seal");

    let (welcome, _tag) = subject.grant(reader_bundle, "follow").expect("grant");

    let mut reader = Vault::open(&root, OFFSET, &reader_key).expect("reader vault");
    let registry = Vault::registry(&[(subject.subject(), subject_bundle.clone())]).expect("registry");
    let welcome = deliver(&store, &subject_key, &welcome).await;
    reader.join(reader_mgr, registry, &subject_bundle, "follow", &welcome).expect("join");

    // More days, after the grant.
    for e in 1..180i64 {
        subject.seal(20_000 + e, &day(20_000 + e)).expect("seal");
    }

    // Reading everything is linear in what is read; reading one day is not.
    let t = Instant::now();
    let all = reader.read_from(20_000).expect("read all");
    let whole = t.elapsed();
    let t = Instant::now();
    let one = reader.read_from(20_179).expect("read newest");
    let newest = t.elapsed();
    assert_eq!(one.len(), 1, "reading from the newest epoch returned {} days", one.len());
    eprintln!(
        "  180 days on disk: all {} days in {:.2}ms, newest day alone in {:.3}ms",
        all.len(),
        whole.as_secs_f64() * 1000.0,
        newest.as_secs_f64() * 1000.0
    );

    // ---- revocation cuts forward, and only forward ----
    let before = reader.read_from(20_000).expect("read all").len();
    subject.revoke(reader.subject()).expect("revoke");
    subject.seal(21_000, &day(21_000)).expect("seal after revoke");

    let after = reader.read_from(20_000).expect("read after revoke");
    assert!(
        !after.contains_key(&21_000),
        "a revoked reader opened a segment sealed after the revocation"
    );
    assert_eq!(after.len(), before, "a revoked reader lost what it already had");
}

fn rand32() -> [u8; 32] {
    use std::time::{SystemTime, UNIX_EPOCH};
    let n = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
    let mut out = [0u8; 32];
    out[..16].copy_from_slice(&n.to_le_bytes());
    out[16..].copy_from_slice(&(n ^ 0x9E37_79B9_7F4A_7C15).to_le_bytes());
    out
}

/// THE PROPERTY THE DECISION RESTS ON: does the recent read stay flat as the
/// history behind it grows?
///
/// A separate vault per history size, because the question is what a reader pays
/// when the subject has been sealing for longer — not what it pays to read more.
#[tokio::test(flavor = "multi_thread")]
async fn reading_the_newest_day_does_not_care_how_much_came_before() {
    let rng = Rng::default();
    eprintln!("  {:>10}  {:>16}", "days held", "newest day");
    for held in [7i64, 30, 90, 180] {
        let subject_key = SigningKey::from_bytes(&rand32());
        let reader_key = SigningKey::from_bytes(&rand32());
        let root = tmp(&format!("depth-{held}"));
        let store = SqliteStoreBuilder::memory().build().await.expect("store");

        let mut subject = Vault::open(&root, OFFSET, &subject_key).expect("vault");
        let (subject_mgr, subject_bundle) = Vault::key_bundle(&rng).expect("bundle");
        let (reader_mgr, reader_bundle) = Vault::key_bundle(&rng).expect("bundle");
        subject.create(subject_mgr).expect("create");
        let (welcome, _tag) = subject.grant(reader_bundle, "follow").expect("grant");

        let mut reader = Vault::open(&root, OFFSET, &reader_key).expect("vault");
        let registry = Vault::registry(&[(subject.subject(), subject_bundle.clone())]).expect("registry");
        let welcome = deliver(&store, &subject_key, &welcome).await;
    reader.join(reader_mgr, registry, &subject_bundle, "follow", &welcome).expect("join");

        for e in 0..held {
            subject.seal(20_000 + e, &day(20_000 + e)).expect("seal");
        }

        let newest = 20_000 + held - 1;
        let t = Instant::now();
        let got = reader.read_from(newest).expect("read");
        let elapsed = t.elapsed();
        assert_eq!(got.len(), 1, "expected exactly the newest day");
        eprintln!("  {held:>10}  {:>13.3}ms", elapsed.as_secs_f64() * 1000.0);
    }
}

/// THE FAILURE THAT WOULD INVALIDATE EVERY GRANT EVER MADE.
///
/// `SecretKey::from_bytes` is `test_utils` only, so an encryption identity
/// cannot be rebuilt from a signing key — it has to be persisted, secrets and
/// all. [D20](../../../docs/decisions.md) records the same hazard for
/// `diaswarm-spaces`, and shadow mode existed partly to catch it: a vault
/// returning from a reboot as a new member is readable by nobody it was ever
/// granted to, and nothing says so.
#[tokio::test(flavor = "multi_thread")]
async fn a_vault_reopens_as_the_same_member() {
    let rng = Rng::default();
    let subject_key = SigningKey::from_bytes(&rand32());
    let reader_key = SigningKey::from_bytes(&rand32());
    let root = tmp("reopen");

    let subject_id;
    let secrets_before;
    {
        let store = SqliteStoreBuilder::memory().build().await.expect("store");
        let mut subject = Vault::open(&root, OFFSET, &subject_key).expect("vault");
        let (subject_mgr, subject_bundle) = Vault::key_bundle(&rng).expect("bundle");
        let (reader_mgr, reader_bundle) = Vault::key_bundle(&rng).expect("bundle");
        subject.create(subject_mgr).expect("create");
        let (welcome, _tag) = subject.grant(reader_bundle, "follow").expect("grant");
        subject.seal(20_000, &day(20_000)).expect("seal");
        subject_id = subject.subject();
        secrets_before = subject.secrets();

        let mut reader = Vault::open(&root, OFFSET, &reader_key).expect("reader");
        let registry = Vault::registry(&[(subject.subject(), subject_bundle.clone())]).expect("registry");
        let welcome = deliver(&store, &subject_key, &welcome).await;
    reader.join(reader_mgr, registry, &subject_bundle, "follow", &welcome).expect("join");
        assert_eq!(reader.read_from(i64::MIN).expect("read").len(), 1);
    }
    // Both vaults dropped. Nothing in memory survives.

    let mut subject = Vault::open(&root, OFFSET, &subject_key).expect("reopen subject");
    assert_eq!(subject.subject(), subject_id, "the subject came back as somebody else");
    assert_eq!(subject.secrets(), secrets_before, "the secret bundle did not survive");

    // The proof that matters: it can still seal under the same group, and a
    // reader that was granted before the restart can still open what follows.
    subject.seal(20_001, &day(20_001)).expect("seal after reopen");

    let reader = Vault::open(&root, OFFSET, &reader_key).expect("reopen reader");
    let got = reader.read_from(i64::MIN).expect("read after reopen");
    assert_eq!(got.len(), 2, "a reader granted before the restart lost access: {:?}", got.keys());

    // And the state file is not world-readable.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(root.join("group.cbor")).expect("state file").permissions().mode();
        assert_eq!(mode & 0o077, 0, "group.cbor is readable by somebody else: {mode:o}");
    }
}

/// THE PROPERTY THE TAG EXISTS FOR: one reader, two subjects, two names.
///
/// [D13](../../../docs/decisions.md) removed the reader's public key from the
/// grant log because it is *the same key in every subject's log* — "one
/// clinician granted by fifty people appeared identically fifty times, which
/// identifies them and clusters their patients". `ControlMessage::Add` publishes
/// whatever the member id is, so this vault gets that property only if the id is
/// per-relationship.
///
/// Asserted rather than assumed, because a benchmark cannot see it and the
/// version of this crate that used a public key passed every other test.
#[test]
fn one_reader_is_a_different_member_to_every_subject() {
    let rng = Rng::default();
    let (_reader_mgr, reader_bundle) = Vault::key_bundle(&rng).expect("reader bundle");

    let mut tags = Vec::new();
    for n in 0..3 {
        let subject_key = SigningKey::from_bytes(&rand32());
        let root = tmp(&format!("unlink-{n}"));
        let mut subject = Vault::open(&root, OFFSET, &subject_key).expect("vault");
        let (subject_mgr, _b) = Vault::key_bundle(&rng).expect("bundle");
        subject.create(subject_mgr).expect("create");
        let (_welcome, tag) = subject.grant(reader_bundle.clone(), "follow").expect("grant");
        tags.push(tag);
    }

    assert_eq!(tags.len(), 3);
    assert_ne!(tags[0], tags[1], "two subjects named the same reader identically");
    assert_ne!(tags[1], tags[2], "two subjects named the same reader identically");
    assert_ne!(tags[0], tags[2], "two subjects named the same reader identically");

    // And the purpose changes the name too, so "clinician" and "follow" are not
    // linkable to each other either.
    let subject_key = SigningKey::from_bytes(&rand32());
    let root = tmp("unlink-purpose");
    let mut subject = Vault::open(&root, OFFSET, &subject_key).expect("vault");
    let (subject_mgr, _b) = Vault::key_bundle(&rng).expect("bundle");
    subject.create(subject_mgr).expect("create");
    let (_w, a) = subject.grant(reader_bundle.clone(), "follow").expect("grant");
    let (_w, b) = subject.grant(reader_bundle, "clinician").expect("grant");
    assert_ne!(a, b, "the same reader under two purposes got one name");
}

/// A SHORT ANSWER MUST NEVER BE A SILENT ONE.
///
/// Three things can stop a segment being read and only one of them is the
/// architecture working. Before `read_reporting` they were indistinguishable
/// from the outside: all three produced "fewer days than you expected" and no
/// way to tell which.
#[tokio::test(flavor = "multi_thread")]
async fn a_read_says_what_it_could_not_open() {
    let rng = Rng::default();
    let subject_key = SigningKey::from_bytes(&rand32());
    let reader_key = SigningKey::from_bytes(&rand32());
    let root = tmp("skipped");
    let store = SqliteStoreBuilder::memory().build().await.expect("store");

    let mut subject = Vault::open(&root, OFFSET, &subject_key).expect("vault");
    let (subject_mgr, subject_bundle) = Vault::key_bundle(&rng).expect("bundle");
    let (reader_mgr, reader_bundle) = Vault::key_bundle(&rng).expect("bundle");
    subject.create(subject_mgr).expect("create");

    // A day before the grant: sealed under a secret the reader never gets.
    subject.seal(20_000, &day(20_000)).expect("seal");
    let (welcome, _tag) = subject.grant(reader_bundle, "follow").expect("grant");
    subject.seal(20_001, &day(20_001)).expect("seal");

    let mut reader = Vault::open(&root, OFFSET, &reader_key).expect("reader");
    let registry = Vault::registry(&[(subject.subject(), subject_bundle.clone())]).expect("registry");
    let welcome = deliver(&store, &subject_key, &welcome).await;
    reader.join(reader_mgr, registry, &subject_bundle, "follow", &welcome).expect("join");

    let (got, skipped) = reader.read_reporting(i64::MIN).expect("read");
    assert!(!got.is_empty(), "the reader opened nothing at all");
    assert_eq!(skipped.lost(), 0, "a read lost data: {skipped:?}");

    // Corrupt a segment on disk and check it is counted rather than vanishing.
    let victim = root.join("segments").join("20001.json");
    let mut raw: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&victim).expect("segment")).expect("json");
    raw["ciphertext"] = serde_json::json!(vec![0u8, 1, 2, 3]);
    std::fs::write(&victim, serde_json::to_vec(&raw).expect("json")).expect("write");

    let (_got, skipped) = reader.read_reporting(i64::MIN).expect("read");
    assert_eq!(
        skipped.undecryptable, 1,
        "a segment that will not decrypt was not reported: {skipped:?}"
    );
    assert!(skipped.lost() > 0, "lost() did not notice real loss");
}

/// Put a control message on the wire and take it off again.
///
/// **THE ONLY WAY INTO A VAULT NOW, AND THAT IS THE POINT.** `join` and
/// `receive` take an `Authentic`, and the only constructor of one is
/// `wire::open_control`. A test that wants to hand a welcome over has to sign
/// it and verify it exactly as the network would, which is what this does.
async fn deliver(
    store: &SqliteStore,
    signing: &SigningKey,
    message: &diaswarm_keys::group::Message,
) -> diaswarm_keys::Authentic {
    wire::publish_control(store, signing, message).await.expect("publish control");
    wire::control_from(store, &signing.verifying_key(), None)
        .await
        .expect("read control")
        .pop()
        .expect("a control message came back")
}

/// REVOKING ONE READER MUST NOT CUT OFF THE OTHERS.
///
/// **THE QUESTION THE DGM BUG RAISED.** A removal rotates the group secret and
/// hands the new one to whoever is still a member — which is read from the
/// subject's own `DgmState`. If that set were not accumulating readers, a
/// rotation would encrypt the new secret towards nobody and every reader would
/// silently stop at the same moment, while the revocation test still passed:
/// "the revoked reader cannot read what came after" is just as true when
/// nobody can.
///
/// So it is asked with two readers, and the second one is the assertion.
#[tokio::test(flavor = "multi_thread")]
async fn revoking_one_reader_leaves_the_other_reading() {
    let rng = Rng::default();
    let subject_key = SigningKey::from_bytes(&rand32());
    let root = tmp("two-readers");
    let store = SqliteStoreBuilder::memory().build().await.expect("store");

    let mut subject = Vault::open(&root, OFFSET, &subject_key).expect("subject");
    let (subject_mgr, subject_bundle) = Vault::key_bundle(&rng).expect("bundle");
    subject.create(subject_mgr).expect("create");

    let mut readers = Vec::new();
    for n in 0..2 {
        let key = SigningKey::from_bytes(&rand32());
        let (mgr, bundle) = Vault::key_bundle(&rng).expect("bundle");
        let (welcome, tag) = subject.grant(bundle, "follow").expect("grant");
        let welcome = deliver(&store, &subject_key, &welcome).await;
        let mut vault = Vault::open(tmp(&format!("reader-{n}")), OFFSET, &key).expect("reader");
        let registry =
            Vault::registry(&[(subject.subject(), subject_bundle.clone())]).expect("registry");
        vault.join(mgr, registry, &subject_bundle, "follow", &welcome).expect("join");
        assert!(vault.is_welcomed(), "reader {n} joined without being welcomed");
        readers.push((vault, tag));
    }

    // A day both can see, to prove they both started from somewhere.
    let before = subject.seal(23_000, &day(23_000)).expect("seal");
    for (reader, _) in &readers {
        assert!(reader.open_segment(&before).is_ok(), "a granted reader could not open a day");
    }

    // ---- revoke the first ------------------------------------------------
    //
    // **A REVOCATION IS NOT ONLY A SUBTRACTION, AND THE REMAINING READERS HAVE
    // TO HEAR IT.** `Group::remove` generates a fresh group secret and encrypts
    // it towards everyone still in the member set, as direct messages inside
    // the control message it returns. A reader that never processes that
    // message never gets the new secret, so it stops opening days at the moment
    // somebody *else* was revoked — and nothing tells it why.
    //
    // The first version of this test asserted the second reader kept reading
    // without delivering the revocation, and it failed. That is the correct
    // failure: publishing the message is not optional bookkeeping, it is how
    // the other readers stay readers.
    let revoked_tag = readers[0].1;
    let revocation = subject.revoke(revoked_tag).expect("revoke");
    let revocation = deliver(&store, &subject_key, &revocation).await;
    readers[1].0.receive(&revocation).expect("the remaining reader takes the revocation");

    let after = subject.seal(23_001, &day(23_001)).expect("seal after revoke");

    assert!(
        readers[0].0.open_segment(&after).is_err(),
        "a revoked reader opened a day sealed after its revocation"
    );
    assert!(
        readers[1].0.open_segment(&after).is_ok(),
        "a reader that processed the revocation still lost access"
    );

    // And the revoked reader is not rescued by being handed the same message.
    let err = readers[0].0.receive(&revocation);
    assert!(
        err.is_err() || readers[0].0.open_segment(&after).is_err(),
        "a revoked reader got the new secret out of its own revocation"
    );
}

/// SEALING A DAY TWICE MUST NOT THROW THE FIRST HALF AWAY.
///
/// **THE BUG NO TEST HERE COULD SEE, BECAUSE NO TEST SEALED AN EPOCH TWICE.**
/// Every test in this file sealed each day once, which is not how a phone uses
/// this: the AAPS plugin accumulates records between drains and flushes them on
/// a five-minute cadence, so one epoch is written to dozens of times as it
/// happens. `seal` was `fs::write` — each flush replaced the day with whatever
/// had arrived since the last one.
///
/// `diaswarm-core`'s seal carries the same comment and the incident behind it:
/// a reader's record count falling from 32,150 to 27,874 between two fetches.
#[test]
fn sealing_a_day_in_pieces_keeps_all_of_it() {
    let rng = Rng::default();
    let subject_key = SigningKey::from_bytes(&rand32());
    let mut subject = Vault::open(tmp("pieces"), OFFSET, &subject_key).expect("vault");
    let (mgr, _bundle) = Vault::key_bundle(&rng).expect("bundle");
    subject.create(mgr).expect("create");

    // A day arriving in twelve flushes, the way a phone delivers one.
    let whole = day(25_000);
    for chunk in whole.chunks(24) {
        subject.seal(25_000, chunk).expect("seal a chunk");
    }

    let got = subject.read_from(25_000).expect("read");
    let back = got.get(&25_000).expect("the day is there");
    assert_eq!(
        back.len(),
        whole.len(),
        "sealing a day in pieces kept {} of {} records",
        back.len(),
        whole.len()
    );
    assert_eq!(
        back.iter().map(|r| r.to_canonical_json()).collect::<Vec<_>>(),
        whole.iter().map(|r| r.to_canonical_json()).collect::<Vec<_>>(),
        "the records came back changed or reordered"
    );

    // And a record offered twice is stored once: a full resync re-drains
    // everything, and the segment must not double.
    subject.seal(25_000, &whole).expect("re-seal the whole day");
    let again = subject.read_from(25_000).expect("read");
    assert_eq!(
        again.get(&25_000).map(Vec::len),
        Some(whole.len()),
        "re-sealing a day it already held doubled it"
    );
}

/// A REVOCATION BITES THE DAY IT HAPPENS IN, NOT THE NEXT ONE.
///
/// **THE REASON THE MERGE RE-SEALS UNDER THE LATEST SECRET.** The cheaper merge
/// keeps the secret the segment already names, and then a reader revoked at
/// noon goes on reading the rest of that day: the segment they can already open
/// is the one still being appended to. Revocation would not bite until
/// midnight.
///
/// The price is stated in `seal` and asserted here: the revoked reader loses
/// the part of *today* it could previously open, and keeps every day that was
/// already finished.
#[tokio::test(flavor = "multi_thread")]
async fn a_revocation_bites_the_day_it_happens_in() {
    let rng = Rng::default();
    let subject_key = SigningKey::from_bytes(&rand32());
    let reader_key = SigningKey::from_bytes(&rand32());
    let root = tmp("bites");
    let store = SqliteStoreBuilder::memory().build().await.expect("store");

    let mut subject = Vault::open(&root, OFFSET, &subject_key).expect("subject");
    let (subject_mgr, subject_bundle) = Vault::key_bundle(&rng).expect("bundle");
    let (reader_mgr, reader_bundle) = Vault::key_bundle(&rng).expect("bundle");
    subject.create(subject_mgr).expect("create");
    let (welcome, tag) = subject.grant(reader_bundle, "follow").expect("grant");
    let welcome = deliver(&store, &subject_key, &welcome).await;

    let mut reader = Vault::open(tmp("bites-reader"), OFFSET, &reader_key).expect("reader");
    let registry = Vault::registry(&[(subject.subject(), subject_bundle.clone())]).expect("reg");
    reader.join(reader_mgr, registry, &subject_bundle, "follow", &welcome).expect("join");

    // A day that finishes, and the first half of the next.
    let closed = subject.seal(26_000, &day(26_000)).expect("seal a closed day");
    let morning = subject.seal(26_001, &day(26_001)[..144]).expect("seal the morning");
    assert!(reader.open_segment(&closed).is_ok(), "the reader could not open a granted day");
    assert!(reader.open_segment(&morning).is_ok(), "the reader could not open the morning");

    // Revoked at noon, and the afternoon is sealed into the same epoch.
    subject.revoke(tag).expect("revoke");
    let afternoon = subject.seal(26_001, &day(26_001)[144..]).expect("seal the afternoon");

    assert!(
        reader.open_segment(&afternoon).is_err(),
        "a reader revoked at noon went on reading the afternoon"
    );
    assert!(
        reader.open_segment(&closed).is_ok(),
        "a revocation reached back into a day that was already finished"
    );

    // And the subject has lost nothing: the whole day is still there for them.
    let mine = subject.read_from(26_001).expect("subject reads its own day");
    assert_eq!(
        mine.get(&26_001).map(Vec::len),
        Some(288),
        "the subject lost records when the day was re-sealed"
    );
}
