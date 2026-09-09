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
use diaswarm_net::wire::{fetch_with, serve_with, Who};
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
fn make_vault(dir: &Path) -> (Identity, Identity, Identity) {
    let subject = Identity::generate();
    let partner = Identity::generate();
    let stranger = Identity::generate();
    let vault = Vault::create(dir, &subject, OFFSET).unwrap();

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
    let (segments, wraps) = fetch_with(addr.clone(), Who::Tag(tag), &got, true).await.expect("fetch");

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
        fetch_with(addr, Who::Tag(their_tag), &theirs, true).await.expect("stranger fetch");
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
    fetch_with(router.endpoint().addr(), Who::Tag(tag), &got, true).await.unwrap();

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
    let vault = Vault::open(&served).unwrap();

    let router = serve_with(served.clone(), SecretKey::generate(), true).await.unwrap();
    let addr = router.endpoint().addr();
    let got = tmp("incr-got");
    let tag = hex(&grant_tag(&partner.encryption, &subject.enc_public(), "follow"));

    let (first, _) = fetch_with(addr.clone(), Who::Tag(tag.clone()), &got, true).await.unwrap();
    assert_eq!(first, 3, "the first sync should fetch everything");

    let (second, _) = fetch_with(addr.clone(), Who::Tag(tag.clone()), &got, true).await.unwrap();
    assert_eq!(second, 0, "nothing changed, so nothing should be re-fetched, got {second}");

    // A new day, and the follower picks it up without re-fetching the rest.
    vault.seal(20_003, &day(20_003, 103.0)).unwrap();
    vault.publish_wraps(&subject, &partner.enc_public(), "follow").unwrap();
    let (third, _) = fetch_with(addr, Who::Tag(tag), &got, true).await.unwrap();
    assert_eq!(third, 1, "a new day should cost exactly one segment, got {third}");

    let opened = Vault::open(&got).unwrap().read_as(&partner, "follow").unwrap();
    assert!(opened.contains_key(&20_003), "the new day did not arrive");
    router.shutdown().await.ok();
}
