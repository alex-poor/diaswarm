//! Does a `diaswarm-keys` subject reach a follower through a stranger?
//!
//! **THE LAST THING D26 LISTED AS NOT BUILT.** The vault was measured, the
//! grant was made signable and sendable, and both were done against a local
//! store: `tests/control.rs` publishes to a `SqliteStore` and reads from the
//! same one. Nothing had crossed a network, and the reader in every test so far
//! shared the subject's directory.
//!
//! This is the swarm property asked of that vault. Three peers, and only one
//! relationship between any two of them:
//!
//!   * the **subject** publishes segments to log 0 and grants to log 1, and
//!     knows nothing about who is listening;
//!   * the **carrier** is a stranger holding a bucket. It is granted nothing,
//!     dials nobody, and ends up with both logs;
//!   * the **follower** never talks to the subject at all. It reads the grant
//!     out of the *carrier's* store, joins from it, and opens the segments it
//!     finds there.
//!
//! **ONE SESSION CARRIES BOTH LOGS**, which is what `KeysArgs` is for: a
//! follower that got segments without grants could not open them, and one that
//! got grants without segments would have nothing to open.

use std::time::Duration;

use diaswarm_core::{EPOCH_MS, Record};
use diaswarm_keys::wire::{self, KeysOperation, CONTROL_LOG_ID, LOG_ID};
use diaswarm_keys::{auth, Vault};
use diaswarm_net::pool;
use diaswarm_net::replicate::KeysReplicator;
use diaswarm_net::swarm::{network_id, Swarm};
use p2panda_core::{Hash, SigningKey, VerifyingKey};
use p2panda_encryption::Rng;
use p2panda_store::logs::LogStore;
use p2panda_store::{SqliteStore, SqliteStoreBuilder};

const OFFSET: i64 = 12 * 3_600_000;

fn tmp(tag: &str) -> std::path::PathBuf {
    let p = std::env::temp_dir().join(format!(
        "diaswarm-keysrepl-{tag}-{}",
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
    ));
    std::fs::create_dir_all(&p).unwrap();
    p
}

fn day(epoch: i64, mgdl: f64) -> Vec<Record> {
    (0..6i64)
        .map(|i| {
            Record::new(epoch * EPOCH_MS + i * 300_000, "cgm")
                .set("mgdl", Some((mgdl + i as f64).into()))
        })
        .collect()
}

/// How many operations of one of this author's logs a store holds.
async fn held(store: &SqliteStore, author: &VerifyingKey, log_id: u32) -> u32 {
    let size = <SqliteStore as LogStore<KeysOperation, VerifyingKey, u32, u32, Hash>>::get_log_size(
        store, author, &log_id, None, None,
    )
    .await
    .unwrap();
    // (OPERATIONS, BYTES) — in that order, whatever the trait's doc says.
    size.map(|(ops, _bytes)| ops).unwrap_or(0)
}

#[tokio::test(flavor = "multi_thread")]
async fn a_follower_gets_its_grant_and_its_days_from_a_stranger() {
    let net = network_id("keys-replicate");
    let rng = Rng::default();

    // ---- the subject ------------------------------------------------------
    let subject_key = SigningKey::generate();
    let mut subject = Vault::open(tmp("subject"), OFFSET, &subject_key).unwrap();
    let (subject_mgr, subject_bundle) = Vault::key_bundle(&rng).unwrap();
    let (reader_mgr, reader_bundle) = Vault::key_bundle(&rng).unwrap();

    let subject_store = SqliteStoreBuilder::memory().build().await.unwrap();
    let create = subject.create(subject_mgr).unwrap();
    wire::publish_control(&subject_store, &subject_key, &create).await.unwrap();
    let (welcome, _tag) = subject.grant(reader_bundle, "follow").unwrap();
    wire::publish_control(&subject_store, &subject_key, &welcome).await.unwrap();

    let days = 3i64;
    for e in 0..days {
        let segment = subject.seal(22_000 + e, &day(22_000 + e, 100.0 + e as f64)).unwrap();
        wire::publish(&subject_store, &subject_key, &segment).await.unwrap();
    }

    let author = subject_key.verifying_key();
    let subject_hex = author.to_hex();

    let publisher = Swarm::join_network(tmp("pub-pool"), SigningKey::generate(), net).await.unwrap();
    let (pub_endpoint, pub_gossip) = publisher.parts();
    let publishing = KeysReplicator::keys(subject_store.clone(), pub_endpoint, pub_gossip)
        .await
        .unwrap();

    // ---- the carrier: a stranger holding a bucket -------------------------
    let carrier_store = SqliteStoreBuilder::memory().build().await.unwrap();
    let carrier = Swarm::join_network(tmp("car-pool"), SigningKey::generate(), net).await.unwrap();
    let (car_endpoint, car_gossip) = carrier.parts();
    let carrying = KeysReplicator::keys(carrier_store.clone(), car_endpoint, car_gossip)
        .await
        .unwrap();

    // The one thing both know: which bucket this subject falls in.
    let depth = pool::depth_for(2);
    let topic = pool::bucket_topic(depth, pool::bucket_of(&subject_hex, depth));
    publishing.carry(topic, &subject_hex).await.unwrap();
    carrying.carry(topic, &subject_hex).await.unwrap();

    let mut segments = 0u32;
    let mut controls = 0u32;
    for _ in 0..60 {
        tokio::time::sleep(Duration::from_secs(1)).await;
        segments = held(&carrier_store, &author, LOG_ID).await;
        controls = held(&carrier_store, &author, CONTROL_LOG_ID).await;
        if segments as i64 >= days && controls >= 2 {
            break;
        }
    }
    println!("  publisher events: {:?}", publishing.events());
    println!("  carrier events:   {:?}", carrying.events());
    assert_eq!(segments as i64, days, "the carrier holds {segments} of {days} segments");
    assert_eq!(controls, 2, "the carrier holds {controls} of 2 control messages");

    // ---- the follower: reads both out of the CARRIER'S store --------------
    //
    // It never contacted the subject. What it has is the subject's public key
    // and key bundle, which is what an invite carries.
    // **THE CHAIN IS CHECKED BEFORE ANYTHING IS ACTED ON**, and against the
    // copy the carrier holds rather than the subject's own — which is the whole
    // point of D13's tamper-evidence replicating. A carrier that had dropped an
    // entry on the way, or a subject that had shown this peer a shorter
    // history, shows up here rather than as a grant that quietly does less than
    // it should.
    let chain = auth::verify_control_chain(&carrier_store, &author).await.unwrap();
    assert!(chain.is_intact(), "the carrier's copy of the grant log broke at {:?}", chain.broken_at);
    assert_eq!(chain.len(), 2, "the carrier holds a different number of grants than it reported");

    let arrived = wire::control_from(&carrier_store, &author, None).await.unwrap();
    assert_eq!(arrived.len(), 2, "both control messages did not authenticate");

    // And the carrier's copy says the same thing as the subject's own. Two
    // peers agreeing is what makes a later disagreement evidence.
    let mine = auth::verify_control_chain(&subject_store, &author).await.unwrap();
    assert_eq!(
        mine.compare(&chain),
        diaswarm_keys::auth::Agreement::Consistent { shared: 2 },
        "the carrier and the subject disagree about the grant log"
    );

    let reader_key = SigningKey::generate();
    let mut reader = Vault::open(tmp("reader"), OFFSET, &reader_key).unwrap();
    let registry = Vault::registry(&[(subject.subject(), subject_bundle.clone())]).unwrap();

    // A READER HAS TO FIND ITS OWN WELCOME. The log holds every control message
    // the subject ever published — the group's creation, and a welcome per
    // grant, each encrypted towards a different tag. Nothing in the message
    // says which is which in clear, and that is the point: a grant that
    // announced who it was for would undo D13. So the reader tries them, and
    // the one addressed to it is the one that opens.
    let mut joined = false;
    for message in &arrived {
        if reader
            .join(reader_mgr.clone(), registry.clone(), &subject_bundle, "follow", message)
            .is_ok()
        {
            joined = true;
            break;
        }
    }
    assert!(joined, "the reader found no welcome it could open among {}", arrived.len());

    // And now the days, from the same stranger's store.
    let carried = wire::segments_tail(&carrier_store, &author, days as u64).await.unwrap();
    assert_eq!(carried.len() as i64, days);
    let mut records = 0usize;
    for segment in &carried {
        let (opened, unparseable) = reader.open_segment(segment).expect("open a carried segment");
        assert_eq!(unparseable, 0);
        records += opened.len();
    }
    assert_eq!(records, (days as usize) * 6, "the follower opened {records} records");

    // ---- and the carrier itself reads none of it --------------------------
    let carrier_key = SigningKey::generate();
    let carrier_vault = Vault::open(tmp("carrier-vault"), OFFSET, &carrier_key).unwrap();
    for segment in &carried {
        assert!(
            carrier_vault.open_segment(segment).is_err(),
            "the carrier opened a segment it was granted nothing for"
        );
    }
}
