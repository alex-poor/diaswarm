//! An always-on pool member, for a machine that does not sleep.
//!
//! **THE POOL'S WEAKNESS IS THAT EVERY HOLDER IS A PHONE.**
//! [D15](../../../docs/decisions.md) promises that a subject whose phone is
//! asleep stays readable, because somebody else holds the same bytes. Every
//! somebody else, so far, is an Android device subject to doze — a condition
//! measured at two hours and counting on 2026-09-14, and one no foreground
//! service turns into "always". One mains-powered peer with a disk makes that
//! promise structural rather than probabilistic.
//!
//! **IT CANNOT READ A BYTE OF WHAT IT HOLDS.** Segments are sealed and this
//! peer is granted nothing; there is no configuration that would make it a
//! server. What it carries is other people's ciphertext, and the deal runs both
//! ways — see the trade table in the README.
//!
//! This is the carrier half of [D29](../../../docs/decisions.md). Reading what
//! *you* were granted, and exporting it, is this binary's next job and needs no
//! new grant semantics; the research gateway needs time-scoped grants and does
//! not exist.

use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result};
use clap::Parser;
use diaswarm_net::replicate::KeysReplicator;
use diaswarm_net::swarm::Swarm;
use p2panda_core::{Hash, VerifyingKey};
use p2panda_store::logs::LogStore;
use p2panda_store::SqliteStore;

/// How often a pass runs.
///
/// **NOT A FRESHNESS KNOB.** Operations arrive by live push between passes
/// (D21), so this is only how often the *share* is recomputed — how quickly
/// this peer notices a subject it ought to be carrying. Freshness is measured
/// elsewhere and is dominated by the sensor, not by this.
const PASS: Duration = Duration::from_secs(60);

#[derive(Parser, Debug)]
#[command(
    name = "diaswarm-peer",
    version,
    about = "Hold a share of the diaswarm pool, so somebody's phone can sleep",
    long_about = None,
)]
struct Args {
    /// Where the node key, follow list and keys.sqlite live.
    ///
    /// Defaults to $XDG_DATA_HOME/diaswarm (or ~/.local/share/diaswarm).
    #[arg(value_name = "DIR")]
    dir: Option<PathBuf>,

    /// How many new strangers to take on per pass.
    ///
    /// `0` carries only what this peer reads, and no strangers.
    ///
    /// It does NOT make this peer a freeloader — reciprocity is unconditional
    /// (D30: if you read it, you carry it), and subjects you follow are always
    /// carried. What `0` switches off is generosity.
    ///
    /// Think twice before using it. Not for secrecy of the data — that is
    /// encryption's job, and a carrier can open none of what it holds — but
    /// because carrying for strangers is what keeps "P holds Y" ambiguous
    /// between following Y and merely holding it. A pool where everyone passed
    /// `0` would publish who reads whom. `0` is for a research gateway, which
    /// must not hold history it was never granted (D5).
    #[arg(long, default_value_t = 4, value_name = "N")]
    adopt: usize,

    /// Join a named pool instead of the default, without a relay. For testing.
    #[arg(long, value_name = "NAME")]
    network: Option<String>,

    /// Do one pass and exit.
    #[arg(long)]
    once: bool,

    /// One JSON object per pass instead of a human-readable line.
    #[arg(long)]
    json: bool,

    /// Print the last N sync events each pass.
    ///
    /// **THE DIAGNOSTIC THAT TOOK A DAY TO BUILD FOR THE PHONES AND WAS NOT
    /// HERE.** "Nothing replicated" has several very different causes — no peer
    /// found, a session that started and failed, a session that finished having
    /// transferred nothing, live mode never starting — and they are
    /// indistinguishable from a count that does not move. This peer stalled at
    /// 1,716 operations while the publisher sealed every minute, and none of
    /// the numbers printed said which of those it was.
    #[arg(long, default_value_t = 0, value_name = "N")]
    events: usize,
}

/// Operations actually in the store for the subjects this peer carries.
///
/// **NOT `Replicator::received()`, AND THE DIFFERENCE IS THE POINT.** That
/// counter increments once per *store event*, so an operation arriving on two
/// topics — which is exactly what the D31 transition does while it carries both
/// the subject topic and the old bucket topic — counts twice. On this machine
/// it read 3,400 against 1,729 actually held. A count that is roughly double
/// the truth looks like health, which is the failure mode this project has been
/// bitten by four times. Ask the store.
async fn held(store: &SqliteStore, subjects: &[String]) -> u64 {
    let mut total = 0u64;
    for hex in subjects {
        let Some(key) = decode_key(hex) else { continue };
        for log_id in diaswarm_keys::wire::LOG_IDS {
            let size = <SqliteStore as LogStore<
                diaswarm_keys::wire::KeysOperation,
                VerifyingKey,
                u32,
                u32,
                Hash,
            >>::get_log_size(store, &key, &log_id, None, None)
            .await
            .unwrap_or(None);
            total += size.map(|(ops, _bytes)| ops as u64).unwrap_or(0);
        }
    }
    total
}

/// A 64-character hex subject as a key.
fn decode_key(s: &str) -> Option<VerifyingKey> {
    if s.len() != 64 {
        return None;
    }
    let mut b = [0u8; 32];
    for (i, out) in b.iter_mut().enumerate() {
        *out = u8::from_str_radix(s.get(i * 2..i * 2 + 2)?, 16).ok()?;
    }
    VerifyingKey::from_bytes(&b).ok()
}

/// Where a peer keeps its state when nobody said.
fn default_dir() -> Result<PathBuf> {
    if let Ok(x) = std::env::var("XDG_DATA_HOME") {
        if !x.is_empty() {
            return Ok(PathBuf::from(x).join("diaswarm"));
        }
    }
    let home = std::env::var("HOME").context("no HOME and no XDG_DATA_HOME; pass a directory")?;
    Ok(PathBuf::from(home).join(".local/share/diaswarm"))
}

/// Bytes on disk under a directory, for saying what this actually costs.
fn disk_bytes(dir: &std::path::Path) -> u64 {
    let Ok(entries) = std::fs::read_dir(dir) else { return 0 };
    entries
        .filter_map(|e| e.ok())
        .map(|e| match e.file_type() {
            Ok(t) if t.is_dir() => disk_bytes(&e.path()),
            Ok(_) => e.metadata().map(|m| m.len()).unwrap_or(0),
            Err(_) => 0,
        })
        .sum()
}

fn human(bytes: u64) -> String {
    const U: [&str; 4] = ["B", "KB", "MB", "GB"];
    let mut v = bytes as f64;
    let mut i = 0;
    while v >= 1024.0 && i < U.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 { format!("{bytes} B") } else { format!("{v:.1} {}", U[i]) }
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let dir = match args.dir {
        Some(d) => d,
        None => default_dir()?,
    };
    std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;

    // The same node key file a phone uses, so a peer keeps the identity it had
    // across restarts. Everything that ranks peers ranks them by it, and a new
    // key reads as one peer leaving and another arriving.
    let key_path = dir.join("node.key");
    let signing = match std::fs::read(&key_path) {
        Ok(b) if b.len() == 32 => {
            p2panda_core::SigningKey::from_bytes(&b.try_into().expect("checked length"))
        }
        _ => {
            let k = p2panda_core::SigningKey::generate();
            std::fs::write(&key_path, k.as_bytes())
                .with_context(|| format!("writing {}", key_path.display()))?;
            k
        }
    };
    let own = signing.verifying_key().to_hex();

    // **THROUGH THE RELAY UNLESS TOLD OTHERWISE.** A carrier that can only be
    // found on one wifi is not a carrier. `--network` exists for tests, which
    // must not dial somebody else's relay.
    let swarm = match &args.network {
        Some(name) => {
            Swarm::join_network(dir.clone(), signing, diaswarm_net::swarm::network_id(name)).await?
        }
        None => Swarm::join(dir.clone(), signing).await?,
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

    let node = swarm.node_id().await?;
    if args.json {
        println!(r#"{{"event":"up","subject":"{own}","node":"{node}","dir":{:?}}}"#, dir.display().to_string());
    } else {
        println!("diaswarm-peer {} — {}", env!("CARGO_PKG_VERSION"), dir.display());
        println!("  node {node}");
        println!("  adopting up to {} new subject(s) a pass; it can read none of them", args.adopt);
    }

    // A stall is only visible against what was held last time.
    let mut was_held = 0u64;
    let mut stalled_for = 0u32;

    loop {
        let tick = swarm.tick().await;
        let share =
            diaswarm_net::share::carry_share(&swarm, &replicator, &dir, &own, args.adopt).await;
        let bytes = disk_bytes(&dir);
        let stored = held(&store, &replicator.carried()).await;
        if stored > was_held {
            stalled_for = 0;
        } else if stored > 0 {
            stalled_for += 1;
        }
        was_held = stored;
        match (tick, share) {
            (Ok(t), Ok(s)) => {
                let pushed = replicator.live_received() > 0;
                if args.json {
                    println!(
                        r#"{{"pool":{},"buckets":{},"carrying":{},"adopted":{},"held":{},"store_events":{},"pushed":{},"bytes":{},"stalled_passes":{}}}"#,
                        t.pool, t.buckets, s.carrying, s.adopted, stored,
                        replicator.received(), pushed, bytes, stalled_for
                    );
                } else {
                    println!(
                        "pool {} · carrying {} (+{}) · holding {} · pushes {} · {}{}",
                        t.pool,
                        s.carrying,
                        s.adopted,
                        stored,
                        if pushed { "yes" } else { "not yet" },
                        human(bytes),
                        if stalled_for >= 3 {
                            format!(" · STALLED {stalled_for} passes — nothing new is arriving")
                        } else {
                            String::new()
                        },
                    );
                }
                // The events, when asked for, or unprompted once a stall is
                // undeniable — because that is the moment somebody needs them.
                if args.events > 0 || stalled_for == 3 {
                    let all = replicator.events();
                    let n = if args.events > 0 { args.events } else { 8 };
                    for line in all.iter().rev().take(n).rev() {
                        println!("    sync: {line}");
                    }
                    if all.is_empty() {
                        println!("    sync: no session events recorded at all");
                    }
                }
            }
            // **SAY WHICH HALF FAILED.** A pass that could not reach the pool
            // and a pass that reached it and carried nothing look identical in
            // a count, and this project has already lost a night to that.
            (Err(e), _) => eprintln!("pool pass failed: {e:#}"),
            (Ok(t), Err(e)) => eprintln!("pool {} reached, but carrying failed: {e:#}", t.pool),
        }
        if args.once {
            return Ok(());
        }
        // **SHUT DOWN WHEN ASKED.** A daemon killed mid-write leaves a SQLite
        // file somebody has to reason about; ^C and `systemctl stop` should
        // both end a pass cleanly rather than in the middle of one.
        tokio::select! {
            _ = tokio::time::sleep(PASS) => {}
            _ = tokio::signal::ctrl_c() => {
                if !args.json {
                    println!("stopping — {} held, {} on disk", replicator.carried().len(), human(bytes));
                }
                return Ok(());
            }
        }
    }
}
