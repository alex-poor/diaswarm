//! One pool peer, in its own process, reporting what it can actually see.
//!
//! **EVERY OTHER TEST IN THIS CRATE RUNS BOTH PEERS IN ONE PROCESS**, which
//! makes gossip trivially local: a `Holding` announcement never crosses a
//! network, and live-mode delivery is a function call away. That is why a
//! thorough pool test passes while two real phones show `holders: none` and
//! `received_live_operations: 0` in every session — the failures live in the
//! gap between processes, and nothing in the suite could see that gap.
//!
//! So this is a peer with a process boundary around it. Two of these, spawned
//! by a test, exchange over the loopback network with real sockets, real
//! gossip and real sync sessions.
//!
//!   poolpeer <dir> <network> <seconds> [subject-to-publish]
//!
//! Prints one JSON line a second on stdout: pool size, who it has heard
//! holding what, and how many operations arrived. A test reads those lines.

use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result};
use diaswarm_core::vault::{hex, Identity, Store, Vault};
use diaswarm_core::{Record, EPOCH_MS};
use diaswarm_net::swarm::{network_id, Swarm};
use p2panda_core::SigningKey;

const OFFSET: i64 = 12 * 3_600_000;

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 4 {
        eprintln!("poolpeer <dir> <network> <seconds> [subject-hex-to-publish]");
        std::process::exit(2);
    }
    let dir = PathBuf::from(&args[1]);
    let net = network_id(&args[2]);
    let seconds: u64 = args[3].parse().context("seconds")?;
    let publish = args.get(4).cloned();

    std::fs::create_dir_all(&dir)?;
    // No relay: this is a loopback test and a relay would make it depend on
    // somebody else's server being up.
    let swarm = Swarm::join_network(dir.clone(), SigningKey::generate(), net)
        .await
        .context("joining")?;

    // A publisher seals a little history so there is something to hold.
    let subject_hex = if publish.is_some() {
        let subject = Identity::generate();
        let store = Store::open(&dir).map_err(|e| anyhow::anyhow!("{e:?}"))?;
        let vdir = store.path_for(&subject.enc_public());
        std::fs::create_dir_all(&vdir)?;
        let vault = Vault::create(&vdir, &subject, OFFSET).map_err(|e| anyhow::anyhow!("{e:?}"))?;
        for i in 0..3 {
            vault
                .seal(
                    24_000 + i,
                    &[Record::new((24_000 + i) * EPOCH_MS + 3_600_000, "cgm")
                        .set("mgdl", Some((100.0 + i as f64).into()))],
                )
                .map_err(|e| anyhow::anyhow!("{e:?}"))?;
        }
        Some(hex(&subject.enc_public()))
    } else {
        None
    };

    let me = swarm.node_id().await?;
    println!(
        "{{\"event\":\"up\",\"node\":\"{me}\",\"subject\":{}}}",
        subject_hex.as_deref().map(|s| format!("\"{s}\"")).unwrap_or_else(|| "null".into())
    );

    let deadline = std::time::Instant::now() + Duration::from_secs(seconds);
    while std::time::Instant::now() < deadline {
        // `tick_and_adopt(1)`: announce, hear, and take on at most one subject.
        // Adoption is what the pool is FOR, and a test that ticks without
        // adopting proves only that peers can see each other.
        let Ok((report, adopted)) = swarm.tick_and_adopt(1).await else {
            tokio::time::sleep(Duration::from_secs(1)).await;
            continue;
        };
        let members = swarm.pool_members().await.map(|m| m.len()).unwrap_or(0);

        // Who have we HEARD holding things — the gossip-borne answer.
        let mut heard_total = 0usize;
        let mut heard_subjects: Vec<String> = Vec::new();
        for (s, _) in report.wanted.iter() {
            let who = swarm.holders_heard(s);
            if !who.is_empty() {
                heard_total += who.len();
                heard_subjects.push(s.clone());
            }
        }
        // And for a subject we were told about on the command line.
        if let Some(s) = std::env::args().nth(5) {
            let who = swarm.holders_heard(&s);
            heard_total += who.len();
            if !who.is_empty() {
                heard_subjects.push(s);
            }
        }

        println!(
            "{{\"event\":\"tick\",\"pool\":{},\"wanted\":{},\"adopted\":{},\"heard\":{},\"depth\":{},\"held\":{}}}",
            report.pool,
            report.wanted.len(),
            adopted,
            heard_total,
            members,
            report.held
        );
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    println!("{{\"event\":\"done\"}}");
    Ok(())
}
