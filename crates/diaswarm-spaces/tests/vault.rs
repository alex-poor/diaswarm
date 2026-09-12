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

/// Records as comparable text, sorted, so two reads differing only in the order
/// they were assembled compare equal.
fn canonical(records: &[Record]) -> Vec<String> {
    let mut v: Vec<String> = records.iter().map(|r| r.to_canonical_json()).collect();
    v.sort();
    v.dedup();
    v
}

/// A deterministic shuffle, so a failure is reproducible from its seed.
fn shuffled<T: Clone>(items: &[T], seed: u64) -> Vec<T> {
    let mut out = items.to_vec();
    let mut state = seed | 1;
    for i in (1..out.len()).rev() {
        // xorshift64*, which is plenty for deciding an order and has no
        // dependency attached to it.
        state ^= state >> 12;
        state ^= state << 25;
        state ^= state >> 27;
        let j = (state.wrapping_mul(0x2545F4914F6CDD1D) >> 33) as usize % (i + 1);
        out.swap(i, j);
    }
    out
}

/// ARRIVAL ORDER IS NOT DEPENDENCY ORDER, AND IT NO LONGER HAS TO BE.
///
/// Log sync happens to deliver in dependency order, which is why nothing here
/// ever tested the alternative. Nothing else guarantees it: a peer hearing one
/// subject from two sources, a sync interrupted halfway, and — the case this
/// was written for — a bundle handed over on a USB stick, where the reader
/// walks a directory in whatever order it gets.
///
/// Upstream states the contract on `SpacesArgs::dependencies()`: "a message
/// should only be processed once all of its dependencies have themselves been
/// processed". Before the orderer, `ingest` processed in arrival order and
/// caught the resulting panic, so an early operation was not delayed — it was
/// dropped, and its records with it.
///
/// **EVERY READER HERE IS GRANTED.** An ungranted vault reads nothing, which is
/// access control working and would make this test pass for the wrong reason.
#[tokio::test]
async fn a_shuffled_bundle_still_reads_completely() {
    let mut subject = peer("shuffle-subject").await;
    let orders: Vec<String> = vec!["in-order".into(), "reversed".into(),
        "seed-1".into(), "seed-7".into(), "seed-99".into()];

    let mut readers = Vec::new();
    for name in &orders {
        readers.push(peer(&format!("shuffle-{name}")).await);
    }

    let mut ops = Vec::new();
    ops.extend(subject.seal(&[]).await.unwrap());
    for r in &readers {
        subject.register(r).await.unwrap();
        r.register(&subject).await.unwrap();
        ops.extend(subject.grant(r.subject(), Reach::Everything).await.unwrap());
    }
    for e in 0..5i64 {
        ops.extend(subject.seal(&day(24_000 + e, 120.0 + e as f64)).await.unwrap());
    }

    let expected = canonical(&readers[0].ingest(&ops).await.unwrap().records);
    assert!(!expected.is_empty(), "the in-order baseline read nothing");

    // Reversed is the worst case an ordering bug can produce: every operation
    // arrives before everything it depends on.
    let permutations: Vec<Vec<_>> = vec![
        ops.iter().rev().cloned().collect(),
        shuffled(&ops, 1),
        shuffled(&ops, 7),
        shuffled(&ops, 99),
    ];

    for (i, permutation) in permutations.into_iter().enumerate() {
        let name = &orders[i + 1];
        let got = readers[i + 1].ingest(&permutation).await.unwrap();
        assert_eq!(got.panicked, 0, "{name}: an operation panicked instead of waiting");
        assert_eq!(
            got.held, 0,
            "{name}: operations still waiting on a dependency that was in the same bundle"
        );
        assert_eq!(
            canonical(&got.records),
            expected,
            "{name}: a reordered bundle read different records from the same bundle in order"
        );
    }
}

/// A BUNDLE THAT ARRIVES IN TWO HALVES, WRONG HALF FIRST.
///
/// The realistic offline shape: somebody copies part of a vault and the rest
/// follows later. The first pass must not lose what it cannot yet place, and
/// the second must complete it without being handed the first half again.
#[tokio::test]
async fn a_dependency_arriving_late_releases_what_waited_for_it() {
    let mut subject = peer("halves-subject").await;
    let baseline = peer("halves-baseline").await;
    let split_reader = peer("halves-split").await;

    let mut ops = Vec::new();
    ops.extend(subject.seal(&[]).await.unwrap());
    for r in [&baseline, &split_reader] {
        subject.register(r).await.unwrap();
        r.register(&subject).await.unwrap();
        ops.extend(subject.grant(r.subject(), Reach::Everything).await.unwrap());
    }
    for e in 0..4i64 {
        ops.extend(subject.seal(&day(25_000 + e, 130.0 + e as f64)).await.unwrap());
    }

    let expected = canonical(&baseline.ingest(&ops).await.unwrap().records);
    assert!(!expected.is_empty(), "the baseline read nothing");

    let (first, second) = ops.split_at(ops.len() / 2);

    // The SECOND half first: most of it depends on operations not here yet.
    let early = split_reader.ingest(&second.to_vec()).await.unwrap();
    assert_eq!(early.panicked, 0, "an out-of-order half panicked instead of waiting");
    assert!(
        early.held > 0,
        "expected the tail of a bundle to be held pending its head, held = {}",
        early.held
    );

    // Now the first half, and nothing from the second is handed over again.
    let late = split_reader.ingest(&first.to_vec()).await.unwrap();
    assert_eq!(late.panicked, 0, "completing the bundle panicked");

    let mut all = early.records;
    all.extend(late.records);
    assert_eq!(
        canonical(&all),
        expected,
        "a bundle delivered in two halves read differently from the same bundle in one"
    );
}

/// AND THIS IS HOW AN INVITE IS MADE ENOUGH — without growing the invite.
///
/// [`a_grant_needs_more_than_the_key_an_invite_carries`] establishes that a
/// public key alone cannot be granted: the spaces vault wants a long-term key
/// bundle too. The tempting fix is a `diaswarm:3:` invite carrying one, and
/// upstream has deliberately closed that door — `Member` is not constructable
/// or serialisable from outside, with a note saying a handle in a struct
/// nobody signed is an impersonation waiting to happen.
///
/// The supported path is a signed operation. A peer publishes its bundle as
/// `SpacesArgs::KeyBundle`; whoever ingests it registers that member as a side
/// effect of processing a message the author signed. The invite still carries
/// only a key — what has to travel is one extra operation, and this project
/// already has a channel for it: the subject dials the follower during the
/// one-scan exchange and the follower hands its invite back.
#[tokio::test]
async fn a_published_key_bundle_makes_a_bare_key_grantable() {
    let mut subject = peer("bundle-subject").await;
    let reader = peer("bundle-reader").await;

    // The key is all an invite carries, and on its own it is not enough.
    let key = reader.subject();
    // Kept, because it opens window 0 and everything later depends on it.
    let opened = subject.seal(&[]).await.unwrap();
    assert!(
        subject.grant(key, Reach::Everything).await.is_err(),
        "a bare key was grantable before the bundle arrived"
    );

    // One signed operation, which is what would travel over the wire.
    let bundle = reader.key_bundle().await.unwrap();
    let ingested = subject.ingest(&[bundle]).await.unwrap();
    assert_eq!(ingested.panicked, 0, "ingesting a key bundle panicked");
    assert_eq!(ingested.held, 0, "a key bundle should depend on nothing");

    // And now the same grant, from the same bare key, works.
    let mut ops = opened;
    ops.extend(subject.grant(key, Reach::Everything).await.expect("grant after bundle"));
    assert!(!ops.is_empty(), "granting produced no operations");

    // The reader can actually read what follows, which is the point of all of it.
    let records = vec![Record::new(26_000 * EPOCH_MS + 3_600_000, "cgm")
        .set("mgdl", Some(140.0.into()))];
    ops.extend(subject.seal(&records).await.unwrap());

    reader.register(&subject).await.unwrap();
    let got = reader.ingest(&ops).await.unwrap();
    assert_eq!(
        got.held, 0,
        "the reader is still waiting on a dependency that should have been in the bundle"
    );
    assert_eq!(got.panicked, 0, "the reader panicked on what it was granted");
    // The stream header rides along with window 0, so this is containment
    // rather than equality — §5.2's `meta` is a record too.
    let read = canonical(&got.records);
    for want in canonical(&records) {
        assert!(
            read.contains(&want),
            "the reader did not get what was sealed after the grant: {want} missing from {read:?}"
        );
    }
}

/// A PERMANENT FOLLOWER WHO ONLY EVER READS A DAY — does the cost reset?
///
/// The parent case, stated exactly: they follow their child until revoked, and
/// they look at the last 24 hours. The cost still grows, because
/// `SpacesArgs::Application` chains to the previous tips of its space and the
/// group's decryption state ratchets forward — so reading today means having
/// processed every operation since the grant, whether or not any of those
/// records are ever displayed.
///
/// The move that should fix it without anyone re-pairing: the **subject** opens
/// a new window and adds the same reader to it. No scan, no new key — the
/// subject already holds their key. If spaces state is per-space, the reader
/// walks the new window from its own beginning and the old one stops costing.
#[tokio::test]
async fn rotating_a_window_resets_what_a_long_standing_reader_pays() {
    let mut subject = peer("rot-subject").await;
    let reader = peer("rot-reader").await;
    subject.register(&reader).await.unwrap();
    reader.register(&subject).await.unwrap();

    let mut ops = Vec::new();
    ops.extend(subject.seal(&[]).await.unwrap());
    ops.extend(subject.grant(reader.subject(), Reach::Everything).await.unwrap());
    // Enough to be in the range where the curve bites: opcost measures 2,000
    // operations at roughly 22 seconds, and one seal is one operation here.
    for e in 0..1500i64 {
        ops.extend(subject.seal(&day(27_000 + e, 100.0 + (e % 80) as f64)).await.unwrap());
    }

    let warm = reader.ingest(&ops).await.unwrap();
    assert!(warm.records.len() > 1, "the reader never got going: {:?}", warm.records.len());
    let warmed = std::time::Instant::now();
    let one_more = subject.seal(&day(27_100, 150.0)).await.unwrap();
    let before = reader.ingest(&one_more).await.unwrap();
    let cost_before = warmed.elapsed();

    // The subject rotates: a new window, the same reader added to it.
    let mut after = subject.grant(reader.subject(), Reach::FromNow).await.unwrap();
    after.extend(subject.seal(&day(27_200, 160.0)).await.unwrap());

    let at = std::time::Instant::now();
    let got = reader.ingest(&after).await.unwrap();
    let cost_after = at.elapsed();

    eprintln!(
        "  warm history {} ops · a day before rotation {:.3}s ({} records, held {})",
        ops.len(), cost_before.as_secs_f64(), before.records.len(), before.held
    );
    eprintln!(
        "  after rotation {:.3}s ({} records, held {})",
        cost_after.as_secs_f64(), got.records.len(), got.held
    );
    assert!(got.records.len() > 0, "the reader read nothing after the rotation");
}
