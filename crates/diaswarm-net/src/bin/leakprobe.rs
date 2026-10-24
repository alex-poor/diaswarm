//! Does repeated session churn (restream) against a STEADY peer leak?
//!
//! **A DELIBERATELY FLAKE-FREE LEAK PROBE.** The process-churn repro (kill and
//! restart a publisher) could not keep two loopback peers in one pool long
//! enough to measure — rediscovery is exactly the "holders: none" pain this
//! crate documents. So this stands both peers up in ONE process on one network
//! id, lets them catch up once, and then exercises the session lifecycle the
//! heap profile implicated: `restream` drops every topic handle and
//! re-subscribes, which is a session teardown + create per call. If p2panda-net
//! frees the old session's state, live heap is flat across N calls; if it does
//! not, it climbs linearly — N times the per-session cost.
//!
//!   leakprobe [iterations] [secs-between]
//!
//! Prints `iter,rss_kb,carrying` once per iteration. Run under jemalloc with
//! `dirty_decay_ms:0` so RSS tracks live heap, and with `lg_prof_interval` so
//! jeprof can attribute any climb.

use std::time::Duration;

use anyhow::Result;
use diaswarm_core::{Record, EPOCH_MS};
use diaswarm_keys::wire::{self, KeysOperation, LOG_ID};
use diaswarm_keys::{Rng, Vault};
use diaswarm_net::pool;
use diaswarm_net::replicate::KeysReplicator;
use diaswarm_net::swarm::{network_id, Swarm};
use p2panda_core::{Hash, SigningKey, VerifyingKey};
use p2panda_store::logs::LogStore;
use p2panda_store::{SqliteStore, SqliteStoreBuilder};

const OFFSET: i64 = 12 * 3_600_000;

fn tmp(tag: &str) -> std::path::PathBuf {
    let p = std::env::temp_dir().join(format!(
        "diaswarm-leakprobe-{tag}-{}",
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
    ));
    std::fs::create_dir_all(&p).unwrap();
    p
}

fn day(epoch: i64, mgdl: f64) -> Vec<Record> {
    (0..6i64)
        .map(|i| Record::new(epoch * EPOCH_MS + i * 300_000, "cgm").set("mgdl", Some((mgdl + i as f64).into())))
        .collect()
}

async fn segments_held(store: &SqliteStore, author: &VerifyingKey) -> u32 {
    let size = <SqliteStore as LogStore<KeysOperation, VerifyingKey, u32, u32, Hash>>::get_log_size(
        store, author, &LOG_ID, None, None,
    )
    .await
    .unwrap_or(None);
    size.map(|(ops, _bytes)| ops).unwrap_or(0)
}

fn rss_kb() -> u64 {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("VmRSS"))
                .and_then(|l| l.split_whitespace().nth(1))
                .and_then(|n| n.parse().ok())
        })
        .unwrap_or(0)
}

#[tokio::main(flavor = "multi_thread")]
async fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    // mode: "live" (restream a steady peer) | "churn" (drop+reform the connection)
    let mode = args.next().unwrap_or_else(|| "live".to_string());
    let iters: usize = args.next().and_then(|s| s.parse().ok()).unwrap_or(60);
    let gap: u64 = args.next().and_then(|s| s.parse().ok()).unwrap_or(3);

    // READ mode: the reader tail-read path Ayni polls, with NO network. Build a
    // realistic log, then call follow::read repeatedly and watch rss. This is
    // the one path the carrier never runs (decrypt), and the tailcost profile
    // put 28% of a phone's cycles in the allocator here.
    if mode == "read" {
        use diaswarm_keys::follow;
        let rng = Rng::default();
        let subject_key = SigningKey::generate();
        let author = subject_key.verifying_key();
        let mut vault = Vault::open(tmp("rsubject"), OFFSET, &subject_key).unwrap();
        let (mgr, _b) = Vault::key_bundle(&rng).unwrap();
        vault.create(mgr).unwrap();
        let store = SqliteStoreBuilder::memory().build().await.unwrap();
        // 14 days x 1440 readings, sealed ~70/segment -> ~288 segments (~20k ops).
        const PER_FLUSH: usize = 70;
        let mut segs = 0usize;
        for d in 0..14i64 {
            let epoch = 22_000 + d;
            let recs: Vec<Record> = (0..1440i64)
                .map(|s| Record::new(epoch * EPOCH_MS + s * 60_000, "cgm").set("mgdl", Some((90.0 + (s as f64 * 3.1) % 140.0).into())))
                .collect();
            for chunk in recs.chunks(PER_FLUSH) {
                let seg = vault.seal_delta(epoch, chunk).unwrap();
                wire::publish(&store, &subject_key, &seg).await.unwrap();
                segs += 1;
            }
        }
        eprintln!("read: built {segs} segments; {iters} full-tail reads");
        println!("iter,rss_kb,records,opened");
        for i in 0..iters {
            let (records, opened, _skipped) =
                follow::read(&vault, &store, &author, 14, 0).await.unwrap();
            println!("{i},{},{},{opened}", rss_kb(), records.len());
            if gap > 0 {
                tokio::time::sleep(Duration::from_millis(gap.min(1) * 50)).await;
            }
        }
        eprintln!("done");
        return Ok(());
    }

    let net = network_id("leakprobe-inproc");
    let rng = Rng::default();

    // Subject publishes; carrier carries the same topic.
    let subject_key = SigningKey::generate();
    let author = subject_key.verifying_key();
    let subject_hex = author.to_hex();
    let mut subject = Vault::open(tmp("subject"), OFFSET, &subject_key).unwrap();
    let (mgr, _bundle) = Vault::key_bundle(&rng).unwrap();

    let subject_store = SqliteStoreBuilder::memory().build().await.unwrap();
    let create = subject.create(mgr).unwrap();
    wire::publish_control(&subject_store, &subject_key, &create).await.unwrap();
    let seg0 = subject.seal(26_000, &day(26_000, 100.0)).unwrap();
    wire::publish(&subject_store, &subject_key, &seg0).await.unwrap();

    // The carrier is the long-lived peer under test (this is the phone/Ayni).
    let carrier_store = SqliteStoreBuilder::memory().build().await.unwrap();
    let carrier = Swarm::join_network(tmp("car"), SigningKey::generate(), net).await.unwrap();
    let (ce, cg) = carrier.parts();
    let carrying = KeysReplicator::keys(carrier_store.clone(), ce, cg).await.unwrap();

    let depth = pool::depth_for(2);
    let topic = pool::bucket_topic(depth, pool::bucket_of(&subject_hex, depth));
    carrying.carry(topic, &subject_hex).await.unwrap();

    // A STABLE publisher node identity, reused every cycle — a phone keeps its
    // node.key across restarts, so a reconnect is the SAME node id with fresh
    // transport, not a new peer.
    let pub_node_bytes = *SigningKey::generate().as_bytes();

    println!("iter,rss_kb,carrying,held");

    if mode == "live" || mode == "stable" {
        // CONVERGENCE TEST ("stable"): one steady session, NO restream churn, NO
        // reconnects. Ops flow at a constant rate (one seg/iter, broadcast once).
        // If RSS tracks `held` (bounded by data), the growth the real carrier
        // shows needs our churn/multi-topic fan-out — i.e. our usage. If RSS
        // climbs far past `held` and never converges on a stable single session,
        // it is a genuine p2panda-net accumulation bug independent of our churn.
        // "live" is the same but ALSO restreams each iter (the churn case).
        let churn = mode == "live";
        let publisher = Swarm::join_network(tmp("pub"), SigningKey::from_bytes(&pub_node_bytes), net)
            .await
            .unwrap();
        let (pe, pg) = publisher.parts();
        let publishing = KeysReplicator::keys(subject_store.clone(), pe, pg).await.unwrap();
        publishing.carry(topic, &subject_hex).await.unwrap();

        let mut caught = 0;
        for _ in 0..60 {
            tokio::time::sleep(Duration::from_secs(1)).await;
            caught = segments_held(&carrier_store, &author).await;
            if caught >= 1 {
                break;
            }
        }
        eprintln!("{mode}: catch-up carrier holds {caught}; {iters} cycles (churn={churn})");
        for i in 0..iters {
            let seg = subject.seal(26_001 + i as i64, &day(26_001 + i as i64, 100.0)).unwrap();
            let op = wire::publish(&subject_store, &subject_key, &seg).await.unwrap();
            let _ = publishing.broadcast(&subject_hex, op);
            if churn {
                let _ = publishing.restream().await;
                let _ = carrying.restream().await;
            }
            tokio::time::sleep(Duration::from_secs(gap)).await;
            let held = segments_held(&carrier_store, &author).await;
            println!("{i},{},{},{held}", rss_kb(), carrying.carried().len());
        }
    } else {
        // CHURN: each cycle stand a publisher up (same node id), let the carrier
        // sync, then DROP it so the connection dies. If the carrier retains the
        // dead session, rss climbs ~linearly in `iter`.
        eprintln!("churn: {iters} connect/drop cycles, gap {gap}s");
        for i in 0..iters {
            {
                let publisher =
                    Swarm::join_network(tmp("pub"), SigningKey::from_bytes(&pub_node_bytes), net)
                        .await
                        .unwrap();
                let (pe, pg) = publisher.parts();
                let publishing = KeysReplicator::keys(subject_store.clone(), pe, pg).await.unwrap();
                publishing.carry(topic, &subject_hex).await.unwrap();
                let seg = subject.seal(26_001 + i as i64, &day(26_001 + i as i64, 100.0)).unwrap();
                let op = wire::publish(&subject_store, &subject_key, &seg).await.unwrap();
                let _ = publishing.broadcast(&subject_hex, op);
                // let the carrier discover + sync this incarnation
                tokio::time::sleep(Duration::from_secs(gap)).await;
                let held = segments_held(&carrier_store, &author).await;
                println!("{i},{},{},{held}", rss_kb(), carrying.carried().len());
                // publisher + its endpoint drop here → connection dies
            }
            // give the carrier a moment to notice the drop before reconnecting
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    }
    eprintln!("done");
    Ok(())
}
