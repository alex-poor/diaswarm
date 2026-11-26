//! Does a fresh carrier's INITIAL INGEST of a large history blow up memory?
//!
//! Reproduces the on-device OOM off-device. On phone B (2026-11-26) a fresh
//! Ayni following the live loop's ~236k-op history went 30 MB -> 3.4 GB in ~40s
//! and died with a native SIGABRT (malloc returning null). The follower's READ
//! is windowed to 6h, so the exploding path is the CARRY/ingest — receiving and
//! storing the whole backlog in one catch-up — not the read.
//!
//! So: a publisher seals DAYS of 1-min readings, then a fresh carrier joins and
//! syncs the lot, and we print the carrier-side RSS every second as `held`
//! climbs from 0 to the full op count. Both stores are on DISK
//! (`open_bounded_store`, exactly what the phone uses) so RSS reflects the sync
//! WORKING SET, not the store held in RAM. A cap stops it before it OOMs the host.
//!
//!   ingestprobe [days]   (default 60; the phone's loop was ~164)

use anyhow::Result;
use diaswarm_core::{Record, EPOCH_MS};
use diaswarm_keys::wire::{self, LOG_ID};
use diaswarm_keys::{open_bounded_store, Rng, Vault};
use diaswarm_net::pool;
use diaswarm_net::replicate::KeysReplicator;
use diaswarm_net::swarm::{network_id, Swarm};
use p2panda_core::{Hash, SigningKey, VerifyingKey};
use p2panda_store::logs::LogStore;
use p2panda_store::SqliteStore;

const OFFSET: i64 = 12 * 3_600_000;
const RSS_CAP_KB: u64 = 8 * 1024 * 1024; // 8 GiB — stop before the host OOMs.

fn tmp(tag: &str) -> std::path::PathBuf {
    let p = std::env::temp_dir().join(format!(
        "diaswarm-ingestprobe-{tag}-{}",
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
    ));
    std::fs::create_dir_all(&p).unwrap();
    p
}

/// A day of 1-minute readings (1440), the real CGM cadence (Libre 3).
fn readings(epoch: i64) -> Vec<Record> {
    (0..1440i64)
        .map(|s| {
            Record::new(epoch * EPOCH_MS + s * 60_000, "cgm")
                .set("mgdl", Some((90.0 + (s as f64 * 3.1) % 140.0).into()))
        })
        .collect()
}

async fn disk_store(tag: &str) -> SqliteStore {
    let dir = tmp(tag);
    let url = format!("sqlite://{}/store.db", dir.display());
    open_bounded_store(&url).await.unwrap()
}

async fn held(store: &SqliteStore, author: &VerifyingKey) -> u32 {
    let size = <SqliteStore as LogStore<p2panda_core::AnyOperation, VerifyingKey, u32, u32, Hash>>::get_log_size(
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
    let days: i64 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(60);
    // readings packed per segment/op. Lower = more OPERATIONS for the same data,
    // which is what the carrier's sync count scales on. 1 = one op per reading.
    let per_flush: usize = std::env::args().nth(2).and_then(|s| s.parse().ok()).unwrap_or(70);
    let net = network_id("ingestprobe-inproc");
    let rng = Rng::default();

    // Publisher with a big history.
    let subject_key = SigningKey::generate();
    let author = subject_key.verifying_key();
    let subject_hex = author.to_hex();
    let mut subject = Vault::open(tmp("subject"), OFFSET, &subject_key).unwrap();
    let (mgr, _bundle) = Vault::key_bundle(&rng).unwrap();
    let subject_store = disk_store("subjectdb").await;
    let create = subject.create(mgr).unwrap();
    wire::publish_control(&subject_store, &subject_key, &create).await.unwrap();

    eprintln!("sealing {days} days (~{} readings)...", days * 1440);
    let mut segs = 0usize;
    for d in 0..days {
        let epoch = 26_000 + d;
        for chunk in readings(epoch).chunks(per_flush) {
            let seg = subject.seal_delta(epoch, chunk).unwrap();
            wire::publish(&subject_store, &subject_key, &seg).await.unwrap();
            segs += 1;
        }
    }
    let total = held(&subject_store, &author).await;
    eprintln!("sealed {segs} segments; subject log holds {total} ops; rss={} MB", rss_kb() / 1024);

    // The carrier under test: a fresh phone/Ayni joining and syncing the lot.
    let carrier_store = disk_store("carrierdb").await;
    let carrier = Swarm::join_network(tmp("car"), SigningKey::generate(), net).await.unwrap();
    let (ce, cg) = carrier.parts();
    let carrying = KeysReplicator::keys(carrier_store.clone(), ce, cg).await.unwrap();
    let depth = pool::depth_for(2);
    let topic = pool::bucket_topic(depth, pool::bucket_of(&subject_hex, depth));
    carrying.carry(topic, &subject_hex).await.unwrap();

    // The publisher serves the backlog.
    let publisher = Swarm::join_network(tmp("pub"), SigningKey::generate(), net).await.unwrap();
    let (pe, pg) = publisher.parts();
    let publishing = KeysReplicator::keys(subject_store.clone(), pe, pg).await.unwrap();
    publishing.carry(topic, &subject_hex).await.unwrap();

    eprintln!("carrier syncing {total} ops; rss by the second (cap {} MB):", RSS_CAP_KB / 1024);
    println!("t_s,rss_kb,held,of");
    let start = std::time::Instant::now();
    let (mut last, mut flat) = (0u32, 0);
    loop {
        let h = held(&carrier_store, &author).await;
        let r = rss_kb();
        println!("{:.0},{},{},{}", start.elapsed().as_secs_f64(), r, h, total);
        if r > RSS_CAP_KB {
            eprintln!("RSS CAP hit at {} MB, held {h}/{total} — the ingest is unbounded", r / 1024);
            break;
        }
        if h >= total {
            eprintln!("CAUGHT UP at rss={} MB (held {h}/{total})", r / 1024);
            break;
        }
        flat = if h == last { flat + 1 } else { 0 };
        if flat > 60 {
            eprintln!("STALLED at held {h}/{total}, rss={} MB", r / 1024);
            break;
        }
        last = h;
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    }
    Ok(())
}
