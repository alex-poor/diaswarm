//! Does a stranger in the pool carry a *keys* subject nobody introduced it to?
//!
//! **THE PROPERTY THE MIGRATION WAS ABOUT TO DROP.** [D15](../../docs/decisions.md)
//! is what makes this a swarm rather than a sync tool: any holder serves
//! identical bytes, so a subject whose phone is asleep is still readable. Pool
//! adoption delivered that for the core vault — `tests/swarm.rs` has a stranger
//! ending up with somebody's ciphertext without being asked — and delivered
//! nothing at all for `diaswarm-keys`, which is the vault being cut over to.
//!
//! `keysCarryAll` carried a phone's own subject and the subjects it follows.
//! Bucket announcements came from `subjects_held`, which reads the core vault's
//! directories, and a keys subject is a different key that appears in none of
//! them. So after a cutover a follower could only ever get data from the
//! subject's own phone: the pool would be full of peers holding nothing of
//! each other's.
//!
//! `tests/keys_replicate.rs` looks like it covers this and does not — there,
//! the carrier is *told* the subject and calls `carry` with it. The whole
//! question here is whether a peer that was told nothing finds out.
//!
//! **TWO PEERS IS A DETERMINISTIC POOL, NOT A LUCKY ONE.** `depth_for(2)` is 1,
//! so there are two buckets, and `REPLICAS` is 3 — more than the pool — so
//! `holders_of_bucket` hands every bucket to both peers. Whichever bucket the
//! subject hashes into is therefore one the carrier holds, every run.

use std::time::Duration;

use diaswarm_core::{EPOCH_MS, Record};
use diaswarm_keys::wire::{self, KeysOperation, LOG_ID};
use diaswarm_keys::Vault;
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
        "diaswarm-keyspool-{tag}-{}",
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

/// Segments of this author's log that a store holds.
async fn segments_held(store: &SqliteStore, author: &VerifyingKey) -> u32 {
    let size = <SqliteStore as LogStore<KeysOperation, VerifyingKey, u32, u32, Hash>>::get_log_size(
        store, author, &LOG_ID, None, None,
    )
    .await
    .unwrap();
    size.map(|(ops, _bytes)| ops).unwrap_or(0)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_stranger_carries_a_keys_subject_it_was_never_introduced_to() {
    let net = network_id("keys-pool-adopt");
    let rng = Rng::default();
    let days = 3i64;

    // ---- the subject: seals, publishes, and tells nobody anything ---------
    let subject_key = SigningKey::generate();
    let author = subject_key.verifying_key();
    let subject_hex = author.to_hex();
    let mut subject = Vault::open(tmp("subject"), OFFSET, &subject_key).unwrap();
    let (mgr, _bundle) = Vault::key_bundle(&rng).unwrap();

    let subject_store = SqliteStoreBuilder::memory().build().await.unwrap();
    let create = subject.create(mgr).unwrap();
    wire::publish_control(&subject_store, &subject_key, &create).await.unwrap();
    for e in 0..days {
        let segment = subject.seal(24_000 + e, &day(24_000 + e, 100.0)).unwrap();
        wire::publish(&subject_store, &subject_key, &segment).await.unwrap();
    }

    let subject_swarm =
        Swarm::join_network(tmp("subject-pool"), SigningKey::generate(), net).await.unwrap();
    let (se, sg) = subject_swarm.parts();
    let publishing = KeysReplicator::keys(subject_store.clone(), se, sg).await.unwrap();

    // A subject serves its own logs: it carries itself, in its own bucket.
    let depth = pool::depth_for(2);
    let own_topic = pool::bucket_topic(depth, pool::bucket_of(&subject_hex, depth));
    publishing.carry(own_topic, &subject_hex).await.unwrap();
    // THE ONE LINE THIS TEST IS ABOUT ON THE SENDING SIDE. Without it the pool
    // never hears that this subject exists.
    subject_swarm.set_keys_held(publishing.carried());

    // ---- the carrier: a stranger, granted nothing, told nothing ----------
    let carrier_store = SqliteStoreBuilder::memory().build().await.unwrap();
    let carrier_swarm =
        Swarm::join_network(tmp("carrier-pool"), SigningKey::generate(), net).await.unwrap();
    let (ce, cg) = carrier_swarm.parts();
    let carrying = KeysReplicator::keys(carrier_store.clone(), ce, cg).await.unwrap();

    // ---- ticking, which is all either peer is ever asked to do ------------
    //
    // Gossip is ephemeral, so both sides tick repeatedly: the subject re-says
    // what it holds, and the carrier acts on whatever it has heard by then.
    let mut adopted: Vec<String> = Vec::new();
    let mut held = 0u32;
    for _ in 0..60 {
        let _ = subject_swarm.tick().await;
        if let Ok(report) = carrier_swarm.tick().await {
            for (subject, _holders) in &report.wanted_keys {
                let topic = pool::bucket_topic(report.depth, pool::bucket_of(subject, report.depth));
                if carrying.carry(topic, subject).await.is_ok() {
                    adopted.push(subject.clone());
                }
            }
            carrier_swarm.set_keys_held(carrying.carried());
        }
        held = segments_held(&carrier_store, &author).await;
        if held as i64 >= days {
            break;
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }

    assert!(
        adopted.contains(&subject_hex),
        "the carrier never heard of the subject. It ticked the pool sixty times \
         and `wanted_keys` never named {subject_hex}: announcements said nothing \
         about the keys vault, which is the state this test exists to end. \
         (heard: {:?})",
        carrier_swarm.keys_holders_heard(&subject_hex)
    );

    assert_eq!(
        held as i64, days,
        "the carrier adopted the subject and holds {held} of {days} segments — \
         it was told which logs to fetch and did not fetch them"
    );

    // ---- and it cannot read a byte of what it is holding ------------------
    let carried = wire::segments_tail(&carrier_store, &author, days as u64).await.unwrap();
    assert_eq!(carried.len() as i64, days);
    let carrier_key = SigningKey::generate();
    let carrier_vault = Vault::open(tmp("carrier-vault"), OFFSET, &carrier_key).unwrap();
    for segment in &carried {
        assert!(
            carrier_vault.open_segment(segment).is_err(),
            "the carrier opened a segment it was granted nothing for"
        );
    }

    // ---- and now it says so, so a third peer could find the subject here --
    //
    // The redundancy is only real if a holder re-announces. A carrier that
    // fetches and stays silent is a dead end: the subject's own phone remains
    // the only address anybody can learn.
    assert!(
        carrier_swarm.keys_holders_heard(&subject_hex).len() >= 1,
        "nobody was recorded as holding {subject_hex}"
    );
    let mut announced_by_carrier = false;
    for _ in 0..30 {
        let _ = carrier_swarm.tick().await;
        let _ = subject_swarm.tick().await;
        if subject_swarm.keys_holders_heard(&subject_hex).len() >= 1 {
            announced_by_carrier = true;
            break;
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    assert!(
        announced_by_carrier,
        "the carrier holds the subject and never announced it, so the pool has \
         one copy that anyone can find and D15 still does not hold"
    );
}
