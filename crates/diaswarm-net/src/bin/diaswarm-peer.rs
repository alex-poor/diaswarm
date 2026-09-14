//! An always-on pool peer, for a machine that does not sleep.
//!
//! **THE POOL'S WEAKNESS IS THAT EVERY HOLDER IS A PHONE.**
//! [D15](../../../docs/decisions.md) promises that a subject whose phone is
//! asleep stays readable, because somebody else holds the same bytes. Every
//! somebody else, so far, is an Android device subject to doze — which is the
//! condition the 2026-09-14 soak exists to measure, and which no amount of
//! foreground service makes into "always". One mains-powered peer with a disk
//! turns that promise from probabilistic into structural, and a year of a
//! subject's share is a few hundred megabytes, which is nothing here.
//!
//! **IT CANNOT READ A BYTE OF WHAT IT HOLDS**, and that is the whole design:
//! segments are sealed, and this peer is granted nothing. It is a volunteer
//! carrier, not a server, and there is no configuration that would make it one.
//!
//!   diaswarm-peer <dir> [--adopt N] [--network NAME] [--once]
//!
//! `<dir>` holds everything: the node key, the follow list, and `keys.sqlite`.
//! `--adopt` is how many new strangers to take on per pass (default 4; `0`
//! means serve what is already held and take on nothing). One line per pass on
//! stdout, so `systemd` or `tee` can keep the record.
//!
//! This is the carrier half of [D29](../../../docs/decisions.md). Reading what
//! you were granted, and exporting it, is the same binary's future job and
//! needs no new grant semantics; the research gateway does, and does not exist.

use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result};
use diaswarm_net::replicate::KeysReplicator;
use diaswarm_net::swarm::Swarm;

/// How often a pass runs. Not a freshness knob: operations arrive by live push
/// between passes (D21), so this is only how often the *share* is recomputed —
/// which is to say how quickly this peer notices a subject it should carry.
const PASS: Duration = Duration::from_secs(60);

fn arg(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned()
}

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let Some(dir) = args.get(1).filter(|d| !d.starts_with("--")).map(PathBuf::from) else {
        eprintln!("usage: diaswarm-peer <dir> [--adopt N] [--network NAME] [--once]");
        std::process::exit(2);
    };
    let adopt: usize = arg(&args, "--adopt").and_then(|v| v.parse().ok()).unwrap_or(4);
    let once = args.iter().any(|a| a == "--once");
    std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;

    // The same node key file a phone uses, so a peer keeps its identity across
    // restarts — everything that ranks peers ranks them by it, and a new key
    // reads as one peer leaving and another arriving.
    let key_path = dir.join("node.key");
    let signing = match std::fs::read(&key_path) {
        Ok(b) if b.len() == 32 => {
            p2panda_core::SigningKey::from_bytes(&b.try_into().unwrap())
        }
        _ => {
            let k = p2panda_core::SigningKey::generate();
            std::fs::write(&key_path, k.as_bytes())?;
            k
        }
    };
    let own = signing.verifying_key().to_hex();

    // Joining through the relay, not `join_network`: this peer exists to be
    // reachable, and a carrier that can only be found on one wifi is not one.
    let swarm = match arg(&args, "--network") {
        Some(name) => {
            Swarm::join_network(dir.clone(), signing.clone(), diaswarm_net::swarm::network_id(&name))
                .await?
        }
        None => Swarm::join(dir.clone(), signing.clone()).await?,
    };

    let url = format!("sqlite://{}", dir.join("keys.sqlite").display());
    let store = diaswarm_keys::SqliteStoreBuilder::new()
        .database_url(&url)
        .create_database(true)
        .build()
        .await
        .map_err(|e| anyhow::anyhow!("opening {url}: {e}"))?;
    let (endpoint, gossip) = swarm.parts();
    let replicator = KeysReplicator::keys(store.clone(), endpoint, gossip).await?;

    println!("peer {own} in the pool as {} — adopting {adopt}/pass", swarm.node_id().await?);

    loop {
        let tick = swarm.tick().await;
        let share = diaswarm_net::share::carry_share(&swarm, &replicator, &dir, &own, adopt).await;
        match (tick, share) {
            (Ok(t), Ok(s)) => println!(
                "pool={} buckets={} carrying={} (+{}) received={} pushed={}",
                t.pool,
                t.buckets,
                s.carrying,
                s.adopted,
                replicator.received(),
                if replicator.live_received() > 0 { "yes" } else { "no" },
            ),
            // **SAY WHICH HALF FAILED.** A pass that could not reach the pool
            // and a pass that reached it and carried nothing look identical in
            // a count, and this project has already lost a night to that.
            (Err(e), _) => println!("pool pass failed: {e}"),
            (Ok(t), Err(e)) => println!("pool={} but carrying failed: {e}", t.pool),
        }
        if once {
            return Ok(());
        }
        tokio::time::sleep(PASS).await;
    }
}
