//! Does turning swarm on actually put your data in a pool?
//!
//! The claim under test is the one the whole project rests on: a peer that
//! nobody told anything ends up holding a stranger's ciphertext, and cannot
//! read it. Every earlier test in this crate proved a weaker thing — that a
//! peer told where to look could fetch. Being told is what a swarm is supposed
//! to remove.

use std::path::Path;
use std::time::Duration;

use diaswarm_core::vault::{hex, Identity, Store, Vault};
use diaswarm_core::{Record, EPOCH_MS};
use diaswarm_net::peer::add_follow;
use diaswarm_net::swarm::{network_id, Swarm};
use p2panda_core::SigningKey;

const OFFSET: i64 = 12 * 3_600_000;

fn tmp(tag: &str) -> std::path::PathBuf {
    let p = std::env::temp_dir().join(format!(
        "diaswarm-swarm-{tag}-{}",
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
    ));
    std::fs::create_dir_all(&p).unwrap();
    p
}

fn day(epoch: i64, marker: f64) -> Vec<Record> {
    vec![Record::new(epoch * EPOCH_MS + 3_600_000, "cgm").set("mgdl", Some(marker.into()))]
}

/// `<id>@<addr>,<addr>` — what a follower would scan off a QR code.
fn upstream(addr: &iroh::EndpointAddr) -> String {
    let ips: Vec<String> = addr.ip_addrs().map(|a| a.to_string()).collect();
    format!("{}@{}", addr.id, ips.join(","))
}

/// A subject with a little history, sealed into a store.
fn subject_in(store_root: &Path) -> (Identity, String) {
    subject_in_granting(store_root, None)
}

/// The same, granting one reader everything.
fn subject_in_granting(store_root: &Path, reader: Option<&Identity>) -> (Identity, String) {
    let subject = Identity::generate();
    let store = Store::open(store_root).unwrap();
    let dir = store.path_for(&subject.enc_public());
    std::fs::create_dir_all(&dir).unwrap();
    let vault = Vault::create(&dir, &subject, OFFSET).unwrap();
    if let Some(r) = reader {
        vault.record_grant(&subject, &r.enc_public(), "follow", "grant", 0).unwrap();
    }
    for i in 0..3 {
        vault.seal(22_000 + i, &day(22_000 + i, 100.0 + i as f64)).unwrap();
    }
    let hex_id = hex(&subject.enc_public());
    (subject, hex_id)
}

/// TURNING IT ON IS THE WHOLE INSTRUCTION.
///
/// Two peers start. One has a subject; the other has nothing and is told
/// nothing — no invite, no address, no subject key. It should find the pool,
/// work out that the subject falls in a bucket it carries, and take a copy.
#[tokio::test(flavor = "multi_thread")]
async fn a_stranger_ends_up_holding_your_ciphertext_without_being_asked() {
    let net = network_id("stranger-holds");
    let publisher_store = tmp("pub");
    let (subject, subject_hex) = subject_in(&publisher_store);

    let holder_store = tmp("holder");

    let publisher = Swarm::join_network(publisher_store.clone(), SigningKey::generate(), net).await.expect("join");
    let holder = Swarm::join_network(holder_store.clone(), SigningKey::generate(), net).await.expect("join");

    // Let discovery do its work. Nobody is given an address.
    let mut wanted: Vec<String> = Vec::new();
    let mut pool_seen = 0;
    for _ in 0..30 {
        tokio::time::sleep(Duration::from_secs(1)).await;
        let _ = publisher.tick().await;
        if let Ok(report) = holder.tick().await {
            pool_seen = report.pool;
            if !report.wanted.is_empty() {
                wanted = report.wanted.iter().map(|(s, _)| s.clone()).collect();
                break;
            }
        }
    }

    assert!(pool_seen >= 2, "the two peers never saw each other: pool was {pool_seen}");
    assert!(
        wanted.contains(&subject_hex),
        "the holder never learned it should be carrying {}: wanted {wanted:?}",
        &subject_hex[..16]
    );

    // And it can actually take a copy, from a peer it was never introduced to.
    let from = publisher.node_id().await.unwrap();
    let (segments, _) = holder.adopt(&subject_hex, &from).await.expect("adopt");
    assert_eq!(segments, 3, "the holder did not take the whole subject");

    // HOLDING IS NOT READING. It has the ciphertext and no way in.
    let copy = Vault::open(&holder_store.join(&subject_hex)).unwrap();
    let stranger = Identity::generate();
    assert!(
        copy.read_as(&stranger, "follow").unwrap().is_empty(),
        "a holder opened data it was never granted"
    );
    // Not even the subject's own reader purpose — nothing was granted at all.
    assert!(copy.read_as(&subject, "follow").unwrap().is_empty());
}

/// A FOLLOWER OUTLIVES THE PHONE IT SCANNED.
///
/// This is the property the whole thing is for, and until now it was only ever
/// demonstrated by handing the follower a second address — first by a person
/// typing one, later by a holder list the subject kept and passed on. That list
/// is gone: it was a second, weaker discovery mechanism beside p2panda's, and
/// the way a home IP address ended up written down on a phone.
///
/// So nobody types anything and nothing records an address. The reader scans
/// ONE code, the subject's. The pool is what tells it where else to look, and
/// what it learns is a node id — dialling is p2panda's problem.
#[tokio::test(flavor = "multi_thread")]
async fn a_follower_survives_the_subject_leaving_without_a_second_address() {
    let reader = Identity::generate();

    let net = network_id("follower-outlives");
    let subject_store = tmp("gone-subject");
    let (subject, subject_hex) = subject_in_granting(&subject_store, Some(&reader));

    let publisher = Swarm::join_network(subject_store.clone(), SigningKey::generate(), net).await.expect("join");
    let relay_store = tmp("gone-relay");
    let relay = Swarm::join_network(relay_store.clone(), SigningKey::generate(), net).await.expect("join");
    let reader_store = tmp("gone-reader");
    let follower = Swarm::join_network(reader_store.clone(), SigningKey::generate(), net).await.expect("join");

    // The one thing anybody is told: the subject's own address, as scanned.
    let subject_addr = upstream(&publisher.iroh_endpoint().await.unwrap().addr());
    add_follow(&reader_store, &subject_hex, &subject_addr, Some("follow")).unwrap();

    // While everyone is up: the relay takes a copy because the pool says it
    // should, and the follower — which only ever refreshes what it follows —
    // hears the relay announce holding it.
    let relay_id = relay.node_id().await.unwrap();
    let mut relay_holds = false;
    let mut heard_relay = false;
    for _ in 0..40 {
        tokio::time::sleep(Duration::from_secs(1)).await;
        let _ = publisher.tick().await;
        if let Ok((_, took)) = relay.tick_and_adopt(4).await {
            relay_holds |= took > 0;
        }
        relay_holds |= relay_store.join(&subject_hex).join("meta.json").exists();
        let refreshed = follower.refresh_follows().await.unwrap();
        assert!(
            refreshed[0].reached(),
            "the follower could not reach the subject at the address it scanned: {:?}",
            refreshed[0].failures
        );
        heard_relay = follower.holders_heard(&subject_hex).contains(&relay_id);
        if relay_holds && heard_relay {
            break;
        }
    }
    assert!(relay_holds, "the relay never took a copy, so there is nothing to fall back to");
    assert!(heard_relay, "the follower never heard that the relay holds {}", &subject_hex[..16]);

    // --- the subject's phone goes away ---
    drop(publisher);
    tokio::time::sleep(Duration::from_secs(2)).await;

    let after = follower.refresh_follows().await.unwrap();
    assert!(
        after[0].reached(),
        "the follower died with the subject: {:?}",
        after[0].failures
    );
    assert_eq!(
        after[0].via.as_deref(),
        Some(relay_id.as_str()),
        "reached, but not through the pool"
    );

    // And what it got is genuinely readable — the relay served openable bytes
    // it cannot open itself.
    let opened =
        Vault::open(&reader_store.join(&subject_hex)).unwrap().read_as(&reader, "follow").unwrap();
    assert_eq!(opened.len(), 3, "the follower could not open what the relay served");
    let stranger = Identity::generate();
    assert!(
        Vault::open(&relay_store.join(&subject_hex))
            .unwrap()
            .read_as(&stranger, "follow")
            .unwrap()
            .is_empty(),
        "the relay could read what it was relaying"
    );
    let _ = subject;
}

/// A SCANNED ADDRESS STILL REACHES A PEER THAT IS IN A POOL.
///
/// This is the guard on a derivation this crate does not own. p2panda hashes
/// the protocol id with its network id before iroh ever sees it, so a peer in
/// the pool does not answer on `diaswarm/5` — it answers on
/// `Hash(diaswarm/5 ++ network_id)`. Dialling with the plain string is refused
/// as "peer doesn't support any known protocol", which looks exactly like the
/// phone being switched off.
///
/// So first contact is a direct dial, deliberately: it is the one path that
/// works before discovery has heard of anybody. If a p2panda upgrade changes
/// the mixing, this fails here rather than as followers that scan a code and
/// silently never connect.
#[tokio::test(flavor = "multi_thread")]
async fn an_address_dial_reaches_a_pooled_peer() {
    let net = network_id("alpn-guard");
    let store = tmp("alpn-pub");
    let (_subject, subject_hex) = subject_in(&store);
    let publisher = Swarm::join_network(store.clone(), SigningKey::generate(), net).await.expect("join");

    let endpoint = publisher.iroh_endpoint().await.unwrap();
    let addr = endpoint.addr();

    let dialer = iroh::Endpoint::bind(iroh::endpoint::presets::N0).await.unwrap();
    let into = tmp("alpn-copy").join(&subject_hex);

    // The plain constant is what an outsider would reasonably try, and it must
    // NOT work — that is the whole reason the derived one has to exist.
    assert!(
        diaswarm_net::wire::fetch_on_alpn(
            &dialer, addr.clone(), diaswarm_net::ALPN, &subject_hex, &into
        )
        .await
        .is_err(),
        "the pool answered on the unmixed ALPN — wire_alpn may no longer be needed"
    );

    let derived = diaswarm_net::swarm::wire_alpn(publisher.network_id());
    let (segments, _) =
        diaswarm_net::wire::fetch_on_alpn(&dialer, addr, &derived, &subject_hex, &into)
            .await
            .expect("a direct dial with the derived ALPN must reach a pooled peer");
    assert_eq!(segments, 3);
}
