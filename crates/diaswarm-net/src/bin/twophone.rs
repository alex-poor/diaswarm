//! Two real phones, one pool, one subject moving between them over log sync.
//!
//! THE LAST CLAIM IN `docs/migration.md` THAT ONLY HARDWARE CAN SETTLE. The
//! whole chain is proven in one process: a subject seals, a stranger carries,
//! a granted reader opens it from the stranger. What that cannot show is two
//! devices on a real network finding each other and moving real bytes — the
//! thing the pool exists for.
//!
//! A binary rather than the plugin, deliberately. The replication path is not
//! wired into AAPS and should not be until it has been watched somewhere it
//! cannot hurt anybody: this runs from `/data/local/tmp`, touches no app, and
//! goes nowhere near a pump.
//!
//! ```sh
//! # on the phone that has data
//! twophone publish /data/local/tmp/tp
//! # on the other one
//! twophone carry /data/local/tmp/tp
//! ```
//!
//! The publisher seals a few days and announces them; the carrier is told
//! nothing but the pool it is in, and reports what it ends up holding — and
//! then tries to read it, which it must not be able to do.

use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use diaswarm_core::{EPOCH_MS, Record};
use diaswarm_net::pool;
use diaswarm_net::replicate::Replicator;
use diaswarm_net::swarm::{Swarm, network_id};
use diaswarm_spaces::Vault;
use p2panda_core::{Hash, Operation, SigningKey, VerifyingKey};
use p2panda_store::logs::LogStore;
use p2panda_store::SqliteStore;

const OFFSET: i64 = 12 * 3_600_000;

/// Both phones must agree on this, and on nothing else.
const POOL: &str = "twophone-check";

fn day(epoch: i64, mgdl: f64) -> Vec<Record> {
    (0..12i64)
        .map(|i| {
            Record::new(epoch * EPOCH_MS + i * 300_000, "cgm")
                .set("mgdl", Some((mgdl + i as f64).into()))
        })
        .collect()
}

async fn held(store: &SqliteStore, author: &VerifyingKey) -> u32 {
    <SqliteStore as LogStore<
        Operation<diaswarm_spaces::SpacesArgs<()>>,
        VerifyingKey,
        u32,
        u32,
        Hash,
    >>::get_log_size(store, author, &diaswarm_spaces::LOG_ID, None, None)
    .await
    .ok()
    .flatten()
    // (OPERATIONS, BYTES), in that order — see the note in tests/replicate.rs.
    .map(|(ops, _bytes)| ops)
    .unwrap_or(0)
}

#[tokio::main]
async fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let role = args.next().unwrap_or_default();
    let root = args.next().unwrap_or_else(|| "/data/local/tmp/tp".to_string());
    let root = std::path::PathBuf::from(root);
    std::fs::create_dir_all(&root)?;

    let net = network_id(POOL);
    let swarm = Swarm::join_network(root.join("pool"), SigningKey::generate(), net)
        .await
        .context("joining the pool")?;
    let (endpoint, gossip) = swarm.parts();
    let vault = Vault::open(root.join("vault"), OFFSET).await.context("opening the vault")?;
    let replicator = Replicator::start(vault.store(), endpoint, gossip).await?;

    println!("  node    {}", &swarm.node_id().await?[..16]);
    println!("  subject {}", &vault.subject().to_hex()[..16]);

    match role.as_str() {
        "publish" => publish(vault, replicator, swarm).await,
        "carry" => carry(vault, replicator, swarm, args.next()).await,
        _ => {
            eprintln!("usage: twophone <publish|carry> [dir] [subject-hex-if-carrying]");
            std::process::exit(2);
        }
    }
}

async fn publish(
    mut vault: Vault,
    replicator: Replicator,
    swarm: Swarm,
) -> Result<()> {
    let subject = vault.subject().to_hex();
    let mut sealed = 0usize;
    for e in 0..3i64 {
        sealed += vault.seal(&day(23_000 + e, 100.0 + e as f64)).await?.len();
    }
    // Window 0 exists now, so the header is in too.
    println!("  sealed  {sealed} operations across 3 days");

    // The bucket this subject falls in, at the depth this pool is at.
    let members = swarm.pool_members().await?.len().max(2);
    let depth = pool::depth_for(members);
    let topic = pool::bucket_topic(depth, pool::bucket_of(&subject, depth));
    replicator.carry(topic, &subject).await?;

    println!("\n  RUN THIS ON THE OTHER PHONE:");
    println!("    twophone carry /data/local/tmp/tp {subject}\n");
    println!("  announcing (ctrl-c to stop)…");
    loop {
        tokio::time::sleep(Duration::from_secs(10)).await;
        let n = swarm.pool_members().await.map(|m| m.len()).unwrap_or(0);
        println!("    pool {n} peers, {} operations sent", replicator.received());
    }
}

async fn carry(
    vault: Vault,
    replicator: Replicator,
    swarm: Swarm,
    subject: Option<String>,
) -> Result<()> {
    let Some(subject) = subject else {
        eprintln!("  the subject's key is needed: twophone carry <dir> <subject-hex>");
        std::process::exit(2);
    };
    let author: VerifyingKey = {
        let bytes: Vec<u8> = (0..subject.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&subject[i..i + 2], 16))
            .collect::<Result<Vec<u8>, _>>()?;
        VerifyingKey::from_bytes(&bytes.try_into().map_err(|_| anyhow::anyhow!("32 bytes"))?)?
    };

    let members = swarm.pool_members().await?.len().max(2);
    let depth = pool::depth_for(members);
    let topic = pool::bucket_topic(depth, pool::bucket_of(&subject, depth));
    replicator.carry(topic, &subject).await?;
    println!("  carrying bucket for {}…", &subject[..16]);

    let started = Instant::now();
    let mut last = 0;
    for _ in 0..60 {
        tokio::time::sleep(Duration::from_secs(2)).await;
        let n = held(&vault.store(), &author).await;
        if n != last {
            println!("    +{:>4.0}s  holding {n} operations", started.elapsed().as_secs_f64());
            last = n;
        }
        if n > 0 && started.elapsed() > Duration::from_secs(20) {
            break;
        }
    }

    println!();
    if last == 0 {
        println!("  NOTHING ARRIVED. The two phones did not replicate.");
        println!("  events: {:?}", replicator.events());
        std::process::exit(1);
    }

    // HOLDING IS NOT READING, on hardware this time.
    let ops = {
        let store = vault.store();
        let entries = <SqliteStore as LogStore<
            Operation<diaswarm_spaces::SpacesArgs<()>>,
            VerifyingKey,
            u32,
            u32,
            Hash,
        >>::get_log_entries(&store, &author, &diaswarm_spaces::LOG_ID, None, None)
        .await?;
        entries
            .map(|e| {
                e.into_iter().map(|(op, _)| diaswarm_spaces::Operation::wrap(op)).collect::<Vec<_>>()
            })
            .unwrap_or_default()
    };
    let read = vault.ingest(&ops).await?;
    println!("  carried {last} operations from a phone it was never introduced to");
    println!(
        "  opened  {} records{}",
        read.records.len(),
        if read.records.is_empty() { " — as it should: it was granted nothing" } else { " — WRONG" }
    );
    if !read.records.is_empty() {
        std::process::exit(1);
    }
    Ok(())
}
