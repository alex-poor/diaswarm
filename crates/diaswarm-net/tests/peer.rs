//! Is it actually a swarm?
//!
//! `transport.rs` proves two peers can move a vault. This proves the property
//! that makes it worth building: a reader gets a subject's history from a peer
//! that is not the subject, has never met the reader, and cannot read a byte
//! of what it is serving — while the subject is off the air entirely.
//!
//! That was demonstrated by hand once, and the demonstration was weaker than
//! it looked: the relaying laptop had replicated the vault while holding the
//! READER's identity, so it happened to fetch the reader's wraps. A real relay
//! knows nobody's tags. Doing it honestly here immediately failed, which is
//! how the protocol came to mirror every wrap rather than one reader's.

use std::path::Path;

use diaswarm_core::vault::{hex, Identity, Store, Vault};
use diaswarm_core::{Record, EPOCH_MS};
use diaswarm_net::peer::{add_follow, add_follow_via, load_follows, refresh_all, refresh_one, Follow};
use diaswarm_net::wire::{fetch_with, serve_with};
use iroh::SecretKey;

const OFFSET: i64 = 12 * 3_600_000;

fn tmp(tag: &str) -> std::path::PathBuf {
    let p = std::env::temp_dir().join(format!(
        "diaswarm-peer-{tag}-{}",
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
    ));
    std::fs::create_dir_all(&p).unwrap();
    p
}

/// `<id>@<addr>,<addr>` — what a peer would be told to dial on a LAN.
fn upstream(addr: &iroh::EndpointAddr) -> String {
    let ips: Vec<String> = addr.ip_addrs().map(|a| a.to_string()).collect();
    format!("{}@{}", addr.id, ips.join(","))
}

fn day(epoch: i64, marker: f64) -> Vec<Record> {
    vec![Record::new(epoch * EPOCH_MS + 3_600_000, "cgm").set("mgdl", Some(marker.into()))]
}

/// A subject with three sealed days, granting one reader everything.
fn subject_with_history(store_root: &Path, reader: &Identity) -> Identity {
    let subject = Identity::generate();
    let store = Store::open(store_root).unwrap();
    let dir = store.path_for(&subject.enc_public());
    std::fs::create_dir_all(&dir).unwrap();
    let vault = Vault::create(&dir, &subject, OFFSET).unwrap();

    vault.record_grant(&subject, &reader.enc_public(), "follow", "grant", 0).unwrap();
    for i in 0..3 {
        vault.seal(21_000 + i, &day(21_000 + i, 100.0 + i as f64)).unwrap();
    }
    subject
}

/// THE SWARM PROPERTY, WITH THE SUBJECT SWITCHED OFF.
///
/// Three parties. The subject seals and grants. A relay — a stranger, granted
/// nothing, holding no key that opens anything — keeps a copy. Then the
/// subject's endpoint shuts down completely, and the reader gets the whole
/// history from the relay.
#[tokio::test(flavor = "multi_thread")]
async fn a_reader_gets_everything_from_a_relay_while_the_subject_is_down() {
    let reader = Identity::generate();

    // --- the subject, serving ---
    let subject_store = tmp("subject-store");
    let subject = subject_with_history(&subject_store, &reader);
    let subject_hex = hex(&subject.enc_public());
    let subject_router = serve_with(subject_store.clone(), SecretKey::generate(), true).await.unwrap();
    let subject_addr = subject_router.endpoint().addr();
    // `id@addr` rather than a bare id: these endpoints run without discovery,
    // which is the same situation as a swarm on a LAN with no internet.
    let subject_eid = upstream(&subject_addr);

    // --- a relay keeps a copy. It is granted nothing. ---
    let relay_store = tmp("relay-store");
    add_follow(&relay_store, &subject_hex, &subject_eid, None).unwrap();
    let got = refresh_all(&relay_store, true).await.unwrap();
    assert_eq!(got.len(), 1);
    assert!(got[0].reached(), "the relay could not reach the subject: {:?}", got[0].failures);
    assert_eq!(got[0].segments, 3, "a relay must take every segment");
    assert_eq!(
        got[0].wraps, 3,
        "a relay must also take wraps it cannot open, or the copy it passes on \
         opens for nobody — which is not a replica of anything"
    );

    // The relay genuinely cannot read what it holds.
    let relay_copy = relay_store.join(&subject_hex);
    let as_relay = Vault::open(&relay_copy).unwrap().read_as(&reader_stranger(), "follow").unwrap();
    assert!(as_relay.is_empty(), "the relay opened something it was never granted");

    // --- the subject goes off the air ---
    subject_router.shutdown().await.unwrap();

    // --- the relay serves; the reader fetches from it ---
    let relay_router = serve_with(relay_store.clone(), SecretKey::generate(), true).await.unwrap();
    let into = tmp("reader");
    let (segments, wraps) =
        fetch_with(relay_router.endpoint().addr(), &subject_hex, &into, true).await.expect("fetch");
    assert_eq!((segments, wraps), (3, 3));

    let opened = Vault::open(&into).unwrap().read_as(&reader, "follow").unwrap();
    assert_eq!(
        opened.keys().copied().collect::<Vec<_>>(),
        vec![21_000, 21_001, 21_002],
        "the reader must get the whole history from a peer that cannot read it"
    );

    // And the subject really was unreachable, so this proves the relay served it.
    let elsewhere = tmp("proof");
    assert!(
        fetch_with(subject_addr, &subject_hex, &elsewhere, true).await.is_err(),
        "the subject answered after being shut down — the test proves nothing"
    );

    relay_router.shutdown().await.unwrap();
}

/// A key that was never granted anything, for asserting a relay is blind.
fn reader_stranger() -> Identity {
    Identity::generate()
}

/// A PEER TRIES EVERY ENDPOINT IT KNOWS.
///
/// The point of holding a list rather than an address: the first peer being
/// asleep is the ordinary case, not the exception, and it is exactly the case
/// the swarm exists to survive.
#[tokio::test(flavor = "multi_thread")]
async fn a_dead_first_endpoint_does_not_stop_a_refresh() {
    let reader = Identity::generate();
    let subject_store = tmp("multi-subject");
    let subject = subject_with_history(&subject_store, &reader);
    let subject_hex = hex(&subject.enc_public());

    // One endpoint that never existed, then one that does.
    let dead = format!("{}@127.0.0.1:1", SecretKey::generate().public());
    let router = serve_with(subject_store.clone(), SecretKey::generate(), true).await.unwrap();
    let live = upstream(&router.endpoint().addr());

    let store = tmp("multi-store");
    let follow = Follow {
        subject: subject_hex.clone(),
        from: vec![dead.clone(), live.clone()],
        purpose: Some("follow".into()),
        relay: None,
        bundle: None,
    };
    let r = refresh_one(&store, &follow, true).await;

    assert_eq!(r.via.as_deref(), Some(live.as_str()), "should have fallen through to the live one");
    assert_eq!(r.segments, 3);
    assert_eq!(r.failures.len(), 1, "the dead endpoint's failure must be kept, not swallowed");
    assert_eq!(r.failures[0].0, dead);

    let opened = Vault::open(&store.join(&subject_hex)).unwrap().read_as(&reader, "follow").unwrap();
    assert_eq!(opened.len(), 3);
    router.shutdown().await.unwrap();
}

/// A SECOND REFRESH MOVES NOTHING.
///
/// A peer refreshes on a timer forever. Mirroring every reader's wraps every
/// two minutes would be a steady, pointless drain on a phone's connection, so
/// the manifest carries a count and an unchanged one is not asked about.
#[tokio::test(flavor = "multi_thread")]
async fn refreshing_an_unchanged_subject_transfers_nothing() {
    let reader = Identity::generate();
    let subject_store = tmp("idle-subject");
    let subject = subject_with_history(&subject_store, &reader);
    let subject_hex = hex(&subject.enc_public());
    let router = serve_with(subject_store.clone(), SecretKey::generate(), true).await.unwrap();
    let eid = upstream(&router.endpoint().addr());

    let store = tmp("idle-store");
    add_follow(&store, &subject_hex, &eid, Some("follow")).unwrap();

    let first = refresh_all(&store, true).await.unwrap();
    assert_eq!((first[0].segments, first[0].wraps), (3, 3));

    let second = refresh_all(&store, true).await.unwrap();
    assert!(second[0].reached());
    assert_eq!(
        (second[0].segments, second[0].wraps),
        (0, 0),
        "nothing changed, so nothing should move"
    );

    // Still fully readable after a no-op refresh.
    let opened = Vault::open(&store.join(&subject_hex)).unwrap().read_as(&reader, "follow").unwrap();
    assert_eq!(opened.len(), 3);
    router.shutdown().await.unwrap();
}

/// A peer that cannot reach anyone says so, per endpoint.
///
/// "The subject looks offline" and "every peer refused" are different problems
/// with different fixes; reporting both as silence is what made a broken wire
/// format look like an unreachable phone for an afternoon.
#[tokio::test(flavor = "multi_thread")]
async fn unreachable_is_reported_per_endpoint() {
    let store = tmp("unreachable");
    let subject = hex(&Identity::generate().enc_public());
    let a = format!("{}@127.0.0.1:1", SecretKey::generate().public());
    let b = format!("{}@127.0.0.1:2", SecretKey::generate().public());
    let follow =
        Follow { subject: subject.clone(), from: vec![a.clone(), b.clone()], purpose: None, relay: None, bundle: None };

    let r = refresh_one(&store, &follow, true).await;
    assert!(!r.reached());
    assert_eq!(r.failures.len(), 2, "every endpoint tried must be accounted for");
    assert!(r.failures.iter().all(|(_, why)| !why.is_empty()), "a failure with no reason is silence");
}

// WHERE THE DISCOVERY TESTS WENT. Three tests lived here: a follower learning
// a second address from a subject's holder list, that list only ever recording
// the authenticated speaker, and only local addresses being advertised. All
// three tested a discovery mechanism this crate no longer has — p2panda's is
// the only one now — and the property they were protecting, a follower
// surviving the subject going away, is tested against the pool in `swarm.rs`.

/// A BUNDLE IS ONLY EVER ADDED, NEVER CLEARED BY SOMEBODY NOT MENTIONING IT.
///
/// **THE FAILURE THIS PREVENTS IS PERMANENT AND SILENT.** A reader needs the
/// subject's keys bundle to derive the tag it was granted under and to open its
/// welcome, and it arrives once, in an invite that is then scanned and thrown
/// away. If a later v2 invite for the same subject — one from a build with no
/// keys vault, or an older code still in a chat thread — were read as "they no
/// longer have a bundle", the reader would lose the one value it cannot get
/// back, and would then fail to join with nothing to say why.
///
/// Rotation is a real change and takes effect. Absence is not a statement.
#[test]
fn a_later_invite_without_a_bundle_does_not_erase_the_one_we_have() {
    let store = tmp("bundle-keep");
    let subject = "11".repeat(32);
    let a = "22".repeat(32);
    let first = "aabbcc00";
    let rotated = "ddeeff11";

    add_follow_via(&store, &subject, &a, Some("follow"), Some(""), Some(first)).unwrap();
    let held = |s: &Path| {
        load_follows(s).unwrap().into_iter().find(|f| f.subject == subject).unwrap().bundle
    };
    assert_eq!(held(&store).as_deref(), Some(first));

    // A v2 invite for the same subject: no bundle in it at all.
    add_follow_via(&store, &subject, &a, Some("follow"), Some(""), None).unwrap();
    assert_eq!(held(&store).as_deref(), Some(first), "an absent bundle erased a stored one");

    // A v3 invite with a different bundle is the subject saying it rotated.
    let changed =
        add_follow_via(&store, &subject, &a, Some("follow"), Some(""), Some(rotated)).unwrap();
    assert!(changed, "a rotated bundle was not recorded as a change");
    assert_eq!(held(&store).as_deref(), Some(rotated));
}
