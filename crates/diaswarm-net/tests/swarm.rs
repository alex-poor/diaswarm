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
use diaswarm_net::swarm::Swarm;
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

/// A subject with a little history, sealed into a store.
fn subject_in(store_root: &Path) -> (Identity, String) {
    let subject = Identity::generate();
    let store = Store::open(store_root).unwrap();
    let dir = store.path_for(&subject.enc_public());
    std::fs::create_dir_all(&dir).unwrap();
    let vault = Vault::create(&dir, &subject, OFFSET).unwrap();
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
    let publisher_store = tmp("pub");
    let (subject, subject_hex) = subject_in(&publisher_store);

    let holder_store = tmp("holder");

    let publisher = Swarm::join(publisher_store.clone(), SigningKey::generate()).await.expect("join");
    let holder = Swarm::join(holder_store.clone(), SigningKey::generate()).await.expect("join");

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
