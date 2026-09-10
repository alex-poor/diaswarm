//! Is this peer reachable from outside its own network?
//!
//! THE QUESTION EVERY DEMONSTRATION SO FAR HAS DODGED. Three phones on one wifi
//! finding each other by mDNS proves nothing — the claim is that a follower
//! reaches a subject *anywhere*. That needs a relay: something with a stable
//! address that both sides can register with, so a node id can be dialled
//! without either party knowing the other's IP.
//!
//! Prints the home relay this peer registered with, and its node id. A peer
//! with no home relay is a peer nobody outside the building can reach.
//!
//!   cargo run -p diaswarm-net --bin relaycheck

use anyhow::Result;
use diaswarm_net::swarm::{DEFAULT_RELAY, Swarm};
use iroh::Watcher;
use p2panda_core::SigningKey;
use std::time::{Duration, Instant};

#[tokio::main]
async fn main() -> Result<()> {
    let dir = std::env::temp_dir().join(format!("relaycheck-{}", std::process::id()));
    println!("  relay configured: {DEFAULT_RELAY}");
    let swarm = Swarm::join(dir, SigningKey::generate()).await?;
    let ep = swarm.iroh_endpoint().await?;
    println!("  node id:          {}", ep.id());

    let started = Instant::now();
    let mut relays = ep.home_relay_status();
    loop {
        let now = relays.get();
        // CONNECTING IS NOT CONNECTED, and reporting the first is how a check
        // ends up passing on a peer nobody can reach.
        let connected = now.iter().any(|r| format!("{r:?}").contains("Connected"));
        if connected {
            println!("\n  HOME RELAY after {:.1}s:", started.elapsed().as_secs_f64());
            for r in now {
                println!("    {r:?}");
            }
            println!("\n  Reachable by node id from any network.");
            return Ok(());
        }
        if started.elapsed() > Duration::from_secs(45) {
            println!("\n  NO HOME RELAY after 45s — this peer is LAN-only.");
            std::process::exit(1);
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}
