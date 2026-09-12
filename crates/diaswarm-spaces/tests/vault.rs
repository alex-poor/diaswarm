//! Does the upstream-backed vault do what the hand-rolled one does?
//!
//! Not "does p2panda work" — `spike/p2panda-spaces` measured that. These are
//! the properties *this project* claims, asked of the crate that would replace
//! `diaswarm-core::vault`, plus the two things the spike could not tell us:
//! whether the shipping API (no `test_utils`) can persist its own state, and
//! whether the records that come out are the records that went in.

use std::path::PathBuf;

use diaswarm_core::{EPOCH_MS, Record};
use diaswarm_spaces::{Reach, Vault};
const OFFSET: i64 = 12 * 3_600_000;

fn tmp(tag: &str) -> PathBuf {
    let p = std::env::temp_dir().join(format!(
        "diaswarm-spaces-{tag}-{}",
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
    ));
    std::fs::create_dir_all(&p).unwrap();
    p
}

fn day(epoch: i64, mgdl: f64) -> Vec<Record> {
    vec![Record::new(epoch * EPOCH_MS + 3_600_000, "cgm").set("mgdl", Some(mgdl.into()))]
}

async fn peer(tag: &str) -> Vault {
    Vault::open(tmp(tag), OFFSET).await.expect("open")
}

/// THE RECORDS THAT COME OUT ARE THE RECORDS THAT WENT IN.
///
/// The differential question, and the reason this crate exists alongside
/// `diaswarm-core` rather than instead of it. A migration that quietly drops a
/// bolus is worse than no migration.
#[tokio::test]
async fn what_is_sealed_is_what_is_read() {
    let mut subject = peer("round-subject").await;
    let reader = peer("round-reader").await;
    subject.register(&reader).await.unwrap();
    reader.register(&subject).await.unwrap();

    let mut sent = Vec::new();
    let mut ops = Vec::new();
    ops.extend(subject.seal(&[]).await.unwrap());
    ops.extend(subject.grant(reader.subject(), Reach::Everything).await.unwrap());

    for e in 0..3i64 {
        let records = day(22_000 + e, 100.0 + e as f64);
        sent.extend(records.clone());
        ops.extend(subject.seal(&records).await.unwrap());
    }

    let got = reader.ingest(&ops).await.unwrap();
    assert_eq!(got.panicked, 0, "an operation panicked, so this history has a hole in it");
    let read = got.records;
    let sent_json: Vec<String> = sent.iter().map(|r| r.to_canonical_json()).collect();
    let read_json: Vec<String> = read.iter().map(|r| r.to_canonical_json()).collect();

    for line in &sent_json {
        assert!(read_json.contains(line), "a sealed record did not come back: {line}");
    }
    assert_eq!(
        read_json.iter().filter(|l| sent_json.contains(l)).count(),
        sent_json.len(),
        "records were duplicated or lost in the round trip"
    );
}

/// A HOLDER CANNOT READ WHAT IT HOLDS.
///
/// The property the whole architecture rests on. Here it is not engineered —
/// the stranger is simply not in the space.
#[tokio::test]
async fn a_stranger_holds_everything_and_reads_nothing() {
    let mut subject = peer("holder-subject").await;
    let reader = peer("holder-reader").await;
    let stranger = peer("holder-stranger").await;
    subject.register(&reader).await.unwrap();
    reader.register(&subject).await.unwrap();
    stranger.register(&subject).await.unwrap();

    let mut ops = subject.seal(&[]).await.unwrap();
    ops.extend(subject.grant(reader.subject(), Reach::Everything).await.unwrap());
    ops.extend(subject.seal(&day(22_100, 117.0)).await.unwrap());

    let by_reader = reader.ingest(&ops).await.unwrap();
    let by_stranger = stranger.ingest(&ops).await.unwrap();
    assert_eq!(by_reader.panicked, 0, "the reader's history has a hole in it");
    assert_eq!(
        by_stranger.panicked, 0,
        "holding a stranger's ciphertext must be uneventful — a peer carries other \
         people's data constantly and cannot panic doing it"
    );
    assert!(!by_reader.records.is_empty(), "the granted reader read nothing");
    assert!(by_stranger.records.is_empty(), "a stranger opened data it was never granted");
}

/// THE CHOICE THE USER ASKED FOR: per grant, with history or without.
///
/// A reader granted "everything" joins window 0, which holds the whole history
/// and keeps receiving. A reader granted "from now on" gets a window of their
/// own, which cannot contain anything published before it existed.
///
/// Nobody is in two windows — a reader can belong to exactly one of a subject's
/// spaces (`spike/p2panda-spaces` §5b) — so what keeps the older reader
/// receiving is `seal` publishing into every window, not membership being
/// carried forward.
#[tokio::test]
async fn a_grant_reaches_back_only_when_it_is_asked_to() {
    let mut subject = peer("reach-subject").await;
    let old = peer("reach-old").await;
    let new = peer("reach-new").await;
    for r in [&old, &new] {
        subject.register(r).await.unwrap();
        r.register(&subject).await.unwrap();
    }

    let mut ops = subject.seal(&[]).await.unwrap();
    ops.extend(subject.seal(&day(22_200, 101.0)).await.unwrap());

    // One reader gets the lot, the other only what comes next.
    ops.extend(subject.grant(old.subject(), Reach::Everything).await.unwrap());
    ops.extend(subject.grant(new.subject(), Reach::FromNow).await.unwrap());
    ops.extend(subject.seal(&day(22_201, 102.0)).await.unwrap());

    let old_in = old.ingest(&ops).await.unwrap();
    let new_in = new.ingest(&ops).await.unwrap();
    assert_eq!(
        (old_in.panicked, new_in.panicked),
        (0, 0),
        "p2panda-auth panicked on an operation, so this reader's history has a hole in it"
    );
    let (old_read, new_read) = (old_in.records, new_in.records);
    let mgdl = |rs: &[Record]| -> Vec<String> { rs.iter().map(|r| r.to_canonical_json()).collect() };

    let before = day(22_200, 101.0)[0].to_canonical_json();
    let after = day(22_201, 102.0)[0].to_canonical_json();

    assert!(mgdl(&old_read).contains(&before), "'everything' did not reach back");
    assert!(mgdl(&old_read).contains(&after), "'everything' stopped receiving");
    assert!(
        !mgdl(&new_read).contains(&before),
        "'from now on' reached back — it read a day sealed before the grant"
    );
    assert!(mgdl(&new_read).contains(&after), "'from now on' received nothing at all");
}

/// Revocation stops the next day, and does not claw back the last one.
#[tokio::test]
async fn revoking_stops_what_comes_next() {
    let mut subject = peer("revoke-subject").await;
    let reader = peer("revoke-reader").await;
    subject.register(&reader).await.unwrap();
    reader.register(&subject).await.unwrap();

    let mut ops = subject.seal(&[]).await.unwrap();
    ops.extend(subject.grant(reader.subject(), Reach::Everything).await.unwrap());
    ops.extend(subject.seal(&day(22_300, 110.0)).await.unwrap());
    ops.extend(subject.revoke(reader.subject()).await.unwrap());
    ops.extend(subject.seal(&day(22_301, 120.0)).await.unwrap());

    let got = reader.ingest(&ops).await.unwrap();
    assert_eq!(got.panicked, 0, "an operation panicked, so this history has a hole in it");
    let read: Vec<String> = got
        .records
        .iter()
        .map(|r| r.to_canonical_json())
        .collect();

    assert!(
        read.contains(&day(22_300, 110.0)[0].to_canonical_json()),
        "revocation took away a day the reader already had — it is prospective only"
    );
    assert!(
        !read.contains(&day(22_301, 120.0)[0].to_canonical_json()),
        "a revoked reader opened a day sealed after the revocation"
    );
}

/// THE GUARD ON A CONSTANT THIS CRATE DOES NOT OWN.
///
/// `p2panda-spaces` files global auth state under `Hash::digest(b"global-groups-context")`
/// and keeps that constant private, while its own state-persisting methods are
/// test-only — so this crate writes the state itself and has to know the key.
/// If upstream changes it, writes would land where the manager does not look
/// and every grant would silently stop persisting.
///
/// So: persist through our path, reopen from disk, and ask the manager's own
/// public API what it finds. Nothing here reaches into internals; if the key
/// drifts, the members list comes back empty and this fails.
#[tokio::test]
async fn state_survives_a_reopen() {
    let root = tmp("reopen");

    let reader = peer("reopen-reader").await;
    let reader_id = reader.subject();

    {
        let mut subject = Vault::open(&root, OFFSET).await.unwrap();
        subject.register(&reader).await.unwrap();
        subject.seal(&[]).await.unwrap();
        subject.grant(reader_id, Reach::Everything).await.unwrap();
        assert_eq!(subject.windows(), 1);
    }

    let reopened = Vault::open(&root, OFFSET).await.unwrap();
    assert_eq!(reopened.windows(), 1, "the window count did not survive");
    assert!(
        reopened.reader_ids().await.unwrap().contains(&reader_id),
        "the grant did not survive a reopen — the auth state was written somewhere \
         p2panda-spaces does not read it from"
    );
}

/// A RECORD PUBLISHED TWICE IS READ ONCE.
///
/// AAPS's sync queue resolves every version row to the record it belongs to, so
/// one record arrives once per version row. The emitter drops repeats within a
/// pass and a bounded drain has many passes, so the same record really does get
/// sealed more than once — 30,188 CGM records published on a real backfill where
/// only 23,247 distinct ones exist.
///
/// `diaswarm-core`'s reader deduplicates for exactly this reason. Without the
/// same thing here, a follower would double-count.
#[tokio::test]
async fn a_record_sealed_twice_is_read_once() {
    let mut subject = peer("dup-subject").await;
    let reader = peer("dup-reader").await;
    subject.register(&reader).await.unwrap();
    reader.register(&subject).await.unwrap();

    let mut ops = subject.seal(&[]).await.unwrap();
    ops.extend(subject.grant(reader.subject(), Reach::Everything).await.unwrap());

    // The same day, sealed three times — which is what several passes handing
    // over the same re-resolved records looks like.
    let d = day(22_500, 133.0);
    for _ in 0..3 {
        ops.extend(subject.seal(&d).await.unwrap());
    }

    let got = reader.ingest(&ops).await.unwrap();
    assert_eq!(got.panicked, 0);
    let readings: Vec<String> = got
        .records
        .iter()
        .filter(|r| r.kind() != "meta")
        .map(|r| r.to_canonical_json())
        .collect();

    assert_eq!(
        readings.len(),
        d.len(),
        "sealed {} records three times and read {} back — a follower would \
         double-count insulin this way",
        d.len(),
        readings.len()
    );
}

/// A GRANT NEEDS MORE THAN THE KEY AN INVITE CARRIES, AND NOTHING SAYS SO.
///
/// **THIS IS A CUTOVER BLOCKER, NOT A CURIOSITY.** Every other test in this
/// file hands `register` the other party's whole [`Vault`] — which exists only
/// because both peers are in one process. A phone has scanned a QR code and
/// holds 64 characters of hex. `diaswarm:2:` carries a subject key, an
/// endpoint, a purpose, a relay and a checksum; it carries no key bundle.
///
/// The shipping vault does not care: `vaultGrant` takes `reader_pub` as hex,
/// `parse_reader` unhexes it, and X25519 goes straight to it. That is why
/// sharing works on hardware today.
///
/// `p2panda-spaces` needs a **long-term key bundle** as well — the public
/// material that lets somebody encrypt to a member who is not online — and
/// upstream is explicit about where it expects that to come from:
/// `Manager::register_member` is documented as taking "key bundle material
/// which was provided through another channel (QR code scan etc.)".
///
/// So on the spaces vault, scanning an invite is not enough to share with
/// anybody. It has not bitten because shadow mode seals and never grants; it
/// would bite on the first real pairing after a cutover. Closing it means an
/// invite that carries a bundle, and something that republishes it when it
/// expires — `Manager` has both the expiry check and the rotation call.
///
/// Asserted rather than merely observed so that whoever fixes the invite
/// format finds out here, at `cargo test`, rather than on a phone.
#[tokio::test]
async fn a_grant_needs_more_than_the_key_an_invite_carries() {
    let mut subject = peer("invite-subject").await;
    let reader = peer("invite-reader").await;

    // Deliberately NOT registered: this is the whole question.
    let key = reader.subject();

    subject.seal(&[]).await.unwrap();
    let err = subject
        .grant(key, Reach::Everything)
        .await
        .expect_err("a bare public key was enough to grant — the invite may now carry a bundle");

    let msg = err.to_string();
    assert!(
        msg.contains("key bundle"),
        "expected the grant to fail for want of a key bundle, got: {msg}"
    );
}
