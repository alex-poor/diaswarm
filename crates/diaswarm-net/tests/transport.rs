//! Does a vault actually move between two peers?
//!
//! Over real iroh endpoints and a real QUIC connection, not a mock. The point
//! is not that the code compiles: it is that a reader on the other end of a
//! network opens exactly what they were granted and nothing else, from bytes
//! they received rather than bytes they already had.

use std::path::Path;

use diaswarm_core::vault::{hex, Identity, Vault};
use diaswarm_core::seal::grant_tag;
use diaswarm_core::{Record, EPOCH_MS};
use diaswarm_net::wire::{fetch_with, have, serve_with, Who};
use diaswarm_core::vault::Store;
use iroh::SecretKey;

const OFFSET: i64 = 12 * 3_600_000;

fn tmp(tag: &str) -> std::path::PathBuf {
    let p = std::env::temp_dir().join(format!(
        "diaswarm-net-{tag}-{}",
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
    ));
    std::fs::create_dir_all(&p).unwrap();
    p
}

fn day(epoch: i64, marker: f64) -> Vec<Record> {
    vec![Record::new(epoch * EPOCH_MS + 3_600_000, "cgm").set("mgdl", Some(marker.into()))]
}

/// A subject with three sealed days, granting one reader from day two.
fn make_vault(store_root: &Path) -> (Identity, Identity, Identity) {
    let subject = Identity::generate();
    let partner = Identity::generate();
    let stranger = Identity::generate();
    let store = Store::open(store_root).unwrap();
    let dir = store.path_for(&subject.enc_public());
    std::fs::create_dir_all(&dir).unwrap();
    let vault = Vault::create(&dir, &subject, OFFSET).unwrap();

    vault.seal(20_000, &day(20_000, 100.0)).unwrap();
    // Granted only from the second segment, so "everything" and "what they were
    // granted" are different numbers and the test can tell them apart.
    vault.record_grant(&subject, &partner.enc_public(), "follow", "grant", 1).unwrap();
    vault.seal(20_001, &day(20_001, 101.0)).unwrap();
    vault.seal(20_002, &day(20_002, 102.0)).unwrap();
    vault.publish_wraps(&subject, &partner.enc_public(), "follow").unwrap();

    (subject, partner, stranger)
}

#[tokio::test(flavor = "multi_thread")]
async fn a_reader_fetches_over_the_network_and_opens_what_it_was_granted() {
    let served = tmp("served");
    let (subject, partner, stranger) = make_vault(&served);

    let router = serve_with(served.clone(), SecretKey::generate(), true)
        .await
        .expect("serve");
    let addr = router.endpoint().addr();

    // --- the partner ------------------------------------------------------
    let got = tmp("partner");
    let tag = hex(&grant_tag(&partner.encryption, &subject.enc_public(), "follow"));
    let (segments, wraps) = fetch_with(addr.clone(), &hex(&subject.enc_public()), Who::Tag(tag), &got, true).await.expect("fetch");

    assert_eq!(segments, 3, "every segment should transfer, readable or not");
    assert_eq!(wraps, 2, "the partner was granted from segment 1, so two wraps");

    let fetched = Vault::open(&got).expect("the fetched vault opens");
    let opened = fetched.read_as(&partner, "follow").expect("read");
    assert_eq!(opened.len(), 2, "opened {} epochs, expected the two granted", opened.len());
    assert!(!opened.contains_key(&20_000), "opened a day it was never granted");
    assert_eq!(opened[&20_001][0].get("mgdl").and_then(|v| v.as_f64()), Some(101.0));

    // --- a stranger, holding the same bytes -------------------------------
    let theirs = tmp("stranger");
    let their_tag = hex(&grant_tag(&stranger.encryption, &subject.enc_public(), "follow"));
    let (segments, wraps) =
        fetch_with(addr, &hex(&subject.enc_public()), Who::Tag(their_tag), &theirs, true).await.expect("stranger fetch");
    assert_eq!(segments, 3, "a stranger gets the ciphertext, by design");
    assert_eq!(wraps, 0, "and no wraps");
    assert!(
        Vault::open(&theirs).unwrap().read_as(&stranger, "follow").unwrap().is_empty(),
        "a stranger opened something after fetching the whole vault"
    );

    router.shutdown().await.ok();
}

#[tokio::test(flavor = "multi_thread")]
async fn the_grant_log_travels_too() {
    // D13: tamper-evidence against the subject rests on other copies existing.
    // A transport that moved only the data would leave the log unverifiable.
    let served = tmp("served-log");
    let (subject, partner, _) = make_vault(&served);

    let router = serve_with(served, SecretKey::generate(), true).await.unwrap();
    let got = tmp("got-log");
    let tag = hex(&grant_tag(&partner.encryption, &subject.enc_public(), "follow"));
    fetch_with(router.endpoint().addr(), &hex(&subject.enc_public()), Who::Tag(tag), &got, true).await.unwrap();

    let fetched = Vault::open(&got).unwrap();
    let grants = fetched.grants().unwrap();
    assert!(!grants.is_empty(), "the grant log did not replicate");
    assert_eq!(
        fetched.verify_chain(&subject.verifying()).unwrap(),
        None,
        "the replicated log does not verify against the subject"
    );
    router.shutdown().await.ok();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_second_sync_fetches_only_what_changed() {
    // A follower syncs every few minutes and almost nothing has changed.
    // Re-fetching the whole history each time to learn that would move
    // megabytes over a phone's connection to discover there was no news.
    let served = tmp("incr");
    let (subject, partner, _) = make_vault(&served);
    let vault = Store::open(&served).unwrap().vault(&subject.enc_public()).unwrap();

    let router = serve_with(served.clone(), SecretKey::generate(), true).await.unwrap();
    let addr = router.endpoint().addr();
    let got = tmp("incr-got");
    let tag = hex(&grant_tag(&partner.encryption, &subject.enc_public(), "follow"));

    let (first, _) = fetch_with(addr.clone(), &hex(&subject.enc_public()), Who::Tag(tag.clone()), &got, true).await.unwrap();
    assert_eq!(first, 3, "the first sync should fetch everything");

    let (second, _) = fetch_with(addr.clone(), &hex(&subject.enc_public()), Who::Tag(tag.clone()), &got, true).await.unwrap();
    assert_eq!(second, 0, "nothing changed, so nothing should be re-fetched, got {second}");

    // A new day, and the follower picks it up without re-fetching the rest.
    vault.seal(20_003, &day(20_003, 103.0)).unwrap();
    vault.publish_wraps(&subject, &partner.enc_public(), "follow").unwrap();
    let (third, _) = fetch_with(addr, &hex(&subject.enc_public()), Who::Tag(tag), &got, true).await.unwrap();
    assert_eq!(third, 1, "a new day should cost exactly one segment, got {third}");

    let opened = Vault::open(&got).unwrap().read_as(&partner, "follow").unwrap();
    assert!(opened.contains_key(&20_003), "the new day did not arrive");
    router.shutdown().await.ok();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_reader_gets_a_subject_from_a_peer_that_is_not_the_subject() {
    // THE SWARM PROPERTY. Everything before this was a personal server: the
    // subject served their own vault and a reader fetched from them, so
    // availability was the subject's phone being awake. Here the subject goes
    // away entirely and a reader still gets their history, from a peer that
    // replicated it and cannot read a word of it.
    let origin = tmp("origin-store");
    let (subject, partner, _) = make_vault(&origin);
    let subject_hex = hex(&subject.enc_public());

    // The subject serves, a middle peer replicates, the subject stops.
    let phone = serve_with(origin.clone(), SecretKey::generate(), true).await.unwrap();
    let relay_store = tmp("relay-store");
    let relay_vault = relay_store.join(&subject_hex);
    let tag = hex(&grant_tag(&partner.encryption, &subject.enc_public(), "follow"));
    fetch_with(phone.endpoint().addr(), &subject_hex, Who::Tag(tag.clone()), &relay_vault, true)
        .await
        .expect("the middle peer replicates");
    phone.shutdown().await.ok();

    // The middle peer now serves what it holds.
    let relay = serve_with(relay_store, SecretKey::generate(), true).await.unwrap();
    let listed = have(relay.endpoint().addr(), true).await.expect("have");
    assert_eq!(listed, vec![subject_hex.clone()], "the peer does not advertise what it holds");

    // A reader that has never spoken to the subject.
    let got = tmp("via-relay");
    let (segments, wraps) =
        fetch_with(relay.endpoint().addr(), &subject_hex, Who::Tag(tag), &got, true)
            .await
            .expect("fetch from the relay");
    assert_eq!(segments, 3);
    assert_eq!(wraps, 2, "the wraps travelled with the vault");

    let opened = Vault::open(&got).unwrap().read_as(&partner, "follow").unwrap();
    assert_eq!(opened.len(), 2, "the subject was offline and the reader still read");
    assert_eq!(opened[&20_001][0].get("mgdl").and_then(|v| v.as_f64()), Some(101.0));

    relay.shutdown().await.ok();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_relaying_peer_cannot_read_what_it_carries() {
    // The property that makes holding other people's data acceptable: a peer
    // that replicates a vault holds ciphertext and wraps addressed to someone
    // else. §7.1 — a holder that cannot read is a holder anyone can be.
    let origin = tmp("origin2");
    let (subject, partner, _) = make_vault(&origin);
    let subject_hex = hex(&subject.enc_public());
    let carrier = Identity::generate();

    let phone = serve_with(origin, SecretKey::generate(), true).await.unwrap();
    let held = tmp("carried").join(&subject_hex);
    let tag = hex(&grant_tag(&partner.encryption, &subject.enc_public(), "follow"));
    fetch_with(phone.endpoint().addr(), &subject_hex, Who::Tag(tag), &held, true).await.unwrap();

    let vault = Vault::open(&held).unwrap();
    assert!(
        vault.read_as(&carrier, "follow").unwrap().is_empty(),
        "the peer carrying this vault could read it"
    );
    assert!(!vault.segments().unwrap().is_empty(), "it is carrying something, though");
    phone.shutdown().await.ok();
}

/// THE FIELD FAILURE, END TO END OVER THE WIRE.
///
/// A subject grants a follower, and then simply keeps looping. No further
/// grant is ever made, because in real use none ever is. Every test above
/// granted after the sealing was finished, which made wrapping-at-grant-time
/// sufficient by construction and hid this for the whole of the build.
///
/// What it looked like: the follower connected, fetched every segment, and
/// opened none of them. 123 segments, 0 wraps, a reading that never changed
/// and no error at any layer to say why.
#[tokio::test(flavor = "multi_thread")]
async fn a_follower_keeps_reading_days_sealed_after_the_grant() {
    let served = tmp("later-days");
    let subject = Identity::generate();
    let partner = Identity::generate();

    let store = Store::open(&served).unwrap();
    let dir = store.path_for(&subject.enc_public());
    std::fs::create_dir_all(&dir).unwrap();
    let vault = Vault::create(&dir, &subject, OFFSET).unwrap();

    // Day one, then the grant — the order a person actually does it in.
    vault.seal(20_500, &day(20_500, 100.0)).unwrap();
    vault.record_grant(&subject, &partner.enc_public(), "follow", "grant", 0).unwrap();
    vault.publish_wraps(&subject, &partner.enc_public(), "follow").unwrap();

    // Then the days after, with nobody granting anything.
    for i in 1..4 {
        vault.seal(20_500 + i, &day(20_500 + i, 100.0 + i as f64)).unwrap();
    }

    let router = serve_with(served.clone(), SecretKey::generate(), true).await.unwrap();
    let addr = router.endpoint().addr();

    let into = tmp("follower");
    let (_segments, wraps) = fetch_with(
        addr,
        &hex(&subject.enc_public()),
        // As a reader, not a precomputed tag: this is the path the follower
        // CLI takes, so it is the path that has to be exercised.
        Who::Reader {
            identity: Identity::from_bytes(&partner.to_bytes()),
            purpose: "follow".into(),
        },
        &into,
        true,
    )
    .await
    .expect("fetch");

    assert_eq!(wraps, 4, "every sealed day must arrive wrapped, not just the granted one");

    let fetched = Vault::open(&into).unwrap();
    let opened = fetched.read_as(&partner, "follow").unwrap();
    assert_eq!(
        opened.keys().copied().collect::<Vec<_>>(),
        vec![20_500, 20_501, 20_502, 20_503],
        "a follower must keep reading after the day they were granted on"
    );
    router.shutdown().await.unwrap();
}
