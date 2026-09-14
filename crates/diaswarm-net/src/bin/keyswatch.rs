//! Watch one subject's keys logs from a laptop, and say how its bytes arrived.
//!
//! **BECAUSE "LIVE MODE DELIVERS NOTHING" TOOK A DAY TO SEE AND AN AFTERNOON TO
//! BELIEVE.** Everything the phones could report was a total — operations
//! received, logs carried, sessions finished — and a total is healthy whether
//! the data came pushed or fetched. The difference is the whole point: pushed
//! means a follower is seconds behind, fetched means it is one sync interval
//! behind and will be for ever.
//!
//! So this is a third peer that is not a phone. It joins the pool, carries one
//! subject, stores nothing it is not given, and prints the two counts
//! separately once a second:
//!
//!   keyswatch <dir> <subject-hex> <seconds> [network-name]
//!
//! ```text
//! {"t":12,"pool":3,"received":41,"live":6,"segments":19,"control":2}
//! ```
//!
//! `live` climbing is the only direct evidence that `SyncHandle::publish` is
//! being called on the publishing side. It was zero in every session on two
//! phones for the life of this project.
//!
//! **IT ADOPTS NOTHING.** It never calls `tick_and_adopt`, so it does not take
//! on strangers' ciphertext just to run a diagnostic. It carries exactly the
//! subject named on the command line.
//!
//! **AND WITH NO ARGUMENT FOR THE NETWORK IT JOINS THE REAL POOL**, which is
//! the point — the phones are in it — but it means this is not something to
//! leave running by accident. `cargo test` builds its own network id for
//! exactly this reason.

use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result};
use diaswarm_keys::wire::{KeysOperation, CONTROL_LOG_ID, LOG_ID};
use diaswarm_net::pool;
use diaswarm_net::replicate::KeysReplicator;
use diaswarm_net::swarm::{default_network, network_id, Swarm};
use p2panda_core::{Hash, SigningKey, VerifyingKey};
use p2panda_store::logs::LogStore;
use p2panda_store::{SqliteStore, SqliteStoreBuilder};

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
    if args.len() < 4 {
        eprintln!("keyswatch <dir> <subject-hex> <seconds> [network-name]");
        std::process::exit(2);
    }
    let dir = PathBuf::from(&args[1]);
    let subject = args[2].to_ascii_lowercase();
    let seconds: u64 = args[3].parse().context("seconds")?;
    let net = match args.get(4) {
        Some(name) => network_id(name),
        None => default_network(),
    };
    let author = {
        let bytes: Vec<u8> = (0..subject.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&subject[i..i + 2], 16))
            .collect::<Result<_, _>>()
            .context("subject is hex")?;
        VerifyingKey::from_bytes(&bytes.try_into().map_err(|_| anyhow::anyhow!("32 bytes"))?)?
    };

    std::fs::create_dir_all(&dir)?;
    let swarm = Swarm::join_network(dir.clone(), SigningKey::generate(), net).await?;
    // The same call the apps make, so this watches through the same store code
    // the phones run and not a convenient stand-in.
    let store = SqliteStoreBuilder::new()
        .database_url(&format!("sqlite://{}", dir.join("keys.sqlite").display()))
        .create_database(true)
        .build()
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    let (endpoint, gossip) = swarm.parts();
    let replicator = KeysReplicator::keys(store.clone(), endpoint, gossip).await?;

    // EVERY PLAUSIBLE DEPTH, because the bucket a subject falls in depends on
    // how many peers the phone thinks are in the pool, and this peer's own
    // count can differ by one at any moment. Carrying four topics instead of
    // one costs a subscription and removes a whole class of "it was watching
    // the wrong bucket" confusion.
    for depth in 0..=3u8 {
        let topic = pool::bucket_topic(depth, pool::bucket_of(&subject, depth));
        replicator.carry(topic, &subject).await?;
    }

    println!(
        "{{\"event\":\"up\",\"node\":\"{}\",\"subject\":\"{subject}\"}}",
        swarm.node_id().await?
    );

    for t in 0..seconds {
        tokio::time::sleep(Duration::from_secs(1)).await;
        let pool_size = swarm.pool_members().await.map(|m| m.len()).unwrap_or(0);
        println!(
            "{{\"t\":{t},\"pool\":{pool_size},\"received\":{},\"live\":{},\"segments\":{},\"control\":{}}}",
            replicator.received(),
            replicator.live_received(),
            held(&store, &author, LOG_ID).await,
            held(&store, &author, CONTROL_LOG_ID).await,
        );
    }
    for e in replicator.events() {
        println!("{{\"event\":\"sync\",\"line\":{:?}}}", e);
    }
    Ok(())
}
