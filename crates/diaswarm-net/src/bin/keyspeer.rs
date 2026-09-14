//! A `diaswarm-keys` publisher or follower, alone in its own process.
//!
//! **BECAUSE THE IN-PROCESS LIVE TEST PROVES THE MECHANISM AND NOT THE
//! TRANSPORT.** `tests/live_mode.rs` runs both peers under one tokio runtime,
//! which is a real test of whether `SyncHandle::publish` is called and whether
//! the far end stores what it is handed — but gossip between two tasks in one
//! process is not gossip between two phones. Every failure that actually cost
//! this project a day lived in the gap between processes: `holders: none`,
//! `received_live_operations: 0`, a follower stale while every internal signal
//! said healthy.
//!
//!   keyspeer <dir> <network> <seconds> publish
//!   keyspeer <dir> <network> <seconds> follow <subject-hex>
//!
//! A publisher seals a segment every two seconds and pushes each one, printing
//! what `broadcast` returned. A follower carries that subject and prints how
//! many operations arrived and how many of those were pushed rather than
//! fetched. One JSON line a second either way; a test reads them.

use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result};
use diaswarm_core::{Record, EPOCH_MS};
use diaswarm_keys::wire;
use diaswarm_keys::Vault;
use diaswarm_net::pool;
use diaswarm_net::replicate::KeysReplicator;
use diaswarm_net::swarm::{network_id, Swarm};
use p2panda_core::{Hash, SigningKey, VerifyingKey};
use diaswarm_keys::Rng;
use diaswarm_keys::wire::KeysOperation;
use p2panda_store::logs::LogStore;
use p2panda_store::{SqliteStore, SqliteStoreBuilder};

mod hex {
    use p2panda_core::VerifyingKey;
    /// A 64-character hex subject as a key, or `None`. Not a parser worth
    /// sharing — `keyswatch` inlines the same three lines.
    pub fn decode_key(s: &str) -> Option<VerifyingKey> {
        if s.len() != 64 {
            return None;
        }
        let mut b = [0u8; 32];
        for (i, out) in b.iter_mut().enumerate() {
            *out = u8::from_str_radix(s.get(i * 2..i * 2 + 2)?, 16).ok()?;
        }
        VerifyingKey::from_bytes(&b).ok()
    }
}

const OFFSET: i64 = 12 * 3_600_000;

fn day(epoch: i64, mgdl: f64) -> Vec<Record> {
    (0..3i64)
        .map(|i| {
            Record::new(epoch * EPOCH_MS + i * 300_000, "cgm")
                .set("mgdl", Some((mgdl + i as f64).into()))
        })
        .collect()
}

/// Distinct operations actually in a log, from the store.
///
/// **THE ONLY COUNT HERE THAT COUNTS THINGS.** `received` increments when an
/// operation is stored and will count the same operation twice if it arrives
/// on two sessions; `live` is p2panda's own and increments more than once per
/// event; `sent` counts topics published on. None of them bounds another, and
/// a test that assumed one did flaked for a day. This asks the store.
async fn held(store: &SqliteStore, author: &VerifyingKey, log_id: u32) -> u32 {
    let size = <SqliteStore as LogStore<KeysOperation, VerifyingKey, u32, u32, Hash>>::get_log_size(
        store, author, &log_id, None, None,
    )
    .await
    .unwrap_or(None);
    size.map(|(ops, _bytes)| ops).unwrap_or(0)
}

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 5 {
        eprintln!("keyspeer <dir> <network> <seconds> publish|follow [subject-hex]");
        std::process::exit(2);
    }
    let dir = PathBuf::from(&args[1]);
    let net = network_id(&args[2]);
    let seconds: u64 = args[3].parse().context("seconds")?;
    let publishing = args[4] == "publish";

    std::fs::create_dir_all(&dir)?;
    let signing = SigningKey::generate();
    let subject_hex = signing.verifying_key().to_hex();

    let swarm = Swarm::join_network(dir.clone(), SigningKey::generate(), net).await?;
    let store = SqliteStoreBuilder::new()
        .database_url(&format!("sqlite://{}", dir.join("keys.sqlite").display()))
        .create_database(true)
        .build()
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    let (endpoint, gossip) = swarm.parts();
    let replicator = KeysReplicator::keys(store.clone(), endpoint, gossip).await?;

    // Whose data this process is about: its own if publishing, somebody else's
    // if following.
    let carried = if publishing {
        subject_hex.clone()
    } else {
        args.get(5).cloned().context("follow needs a subject")?.to_ascii_lowercase()
    };

    // EVERY PLAUSIBLE DEPTH. The bucket a subject falls in depends on how many
    // peers each side thinks are in the pool, and two processes starting
    // seconds apart can disagree by one for a while. Carrying all four removes
    // a class of "it was listening to the wrong bucket" from the test, which is
    // noise and not the property under test.
    for depth in 0..=3u8 {
        let topic = pool::bucket_topic(depth, pool::bucket_of(&carried, depth));
        replicator.carry(topic, &carried).await?;
    }

    let mut vault = if publishing {
        let mut v = Vault::open(dir.join("vault"), OFFSET, &signing)?;
        let (mgr, _bundle) = Vault::key_bundle(&Rng::default())?;
        let create = v.create(mgr)?;
        let op = wire::publish_control(&store, &signing, &create).await?;
        replicator.broadcast(&subject_hex, op);
        Some(v)
    } else {
        None
    };

    println!(
        "{{\"event\":\"up\",\"role\":\"{}\",\"subject\":\"{carried}\",\"node\":\"{}\"}}",
        if publishing { "publish" } else { "follow" },
        swarm.node_id().await?
    );

    let mut epoch = 25_000i64;
    // Cumulative, so a test can compare what the publisher SENT against what
    // the follower says it received pushed. A follower reporting more live
    // arrivals than the publisher ever pushed is the defect this measures.
    let mut pushed_total = 0usize;
    // **OPERATIONS, WHICH IS THE ONLY THING A FOLLOWER'S STORE IS COMPARABLE
    // TO.** `sent` above counts what `broadcast` returned — topics published
    // on — and `live` counts p2panda's own increments, measured at one per
    // event on one build and two on another. Neither is a count of operations,
    // so neither can bound the other. This one is: the publisher created
    // exactly this many, and no follower can store more than exist.
    let mut ops_total = 1usize; // the create control message, published above
    for t in 0..seconds {
        tokio::time::sleep(Duration::from_secs(1)).await;
        let pool_size = swarm.pool_members().await.map(|m| m.len()).unwrap_or(0);

        let mut pushed = 0usize;
        if let Some(v) = vault.as_mut() {
            // **AFTER THE FOLLOWER HAS HAD TIME TO CATCH UP**, so an arrival
            // counted as live cannot be a catch-up in disguise. The first ten
            // seconds are for discovery and the sync phase.
            if t >= 10 {
                let segment = v.seal(epoch, &day(epoch, 100.0))?;
                let op = wire::publish(&store, &signing, &segment).await?;
                pushed = replicator.broadcast(&subject_hex, op);
                pushed_total += pushed;
                ops_total += 1;
                epoch += 1;
            }
        }

        let (received, live) = (replicator.received(), replicator.live_received());
        // The subject THIS process is about — its own when publishing,
        // somebody else's when following.
        let segments = match hex::decode_key(&carried) {
            Some(k) => held(&store, &k, wire::LOG_ID).await,
            None => 0,
        };
        println!(
            "{{\"t\":{t},\"pool\":{pool_size},\"received\":{received},\"live\":{live},\"pushed\":{pushed},\"sent\":{pushed_total},\"ops\":{ops_total},\"segments\":{segments}}}"
        );
    }
    for e in replicator.events() {
        println!("{{\"event\":\"sync\",\"line\":{e:?}}}");
    }
    Ok(())
}
