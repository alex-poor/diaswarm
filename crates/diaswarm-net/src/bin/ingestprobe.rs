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
//!   ingestprobe [days] [per_flush] [keep] [window]
//!     days      history to seal (default 60; the phone's loop was ~164)
//!     per_flush readings packed per op; lower = more OPS for the same data
//!               (default 70; 1 = one op per reading)
//!     keep      >0: PUBLISHER prunes its segment log to the last `keep` ops
//!               before the carrier syncs — the D35 fix (publisher-side).
//!     window    >0: CARRIER advertises a windowed `Have` and syncs only the
//!               last `window` segment ops EVEN against a full publisher log —
//!               the follower-side lever (step 2). See `WindowedStore`.
//!
//! The money shot is `ingestprobe 60 2 0 2000`: 43,200 ops served WHOLE, carrier
//! windowed to 2,000 → it pulls ~2,000 and RSS stays flat, where the same run
//! WITHOUT the window (`… 0 0`) climbs and stalls. That is the proof that the
//! fix belongs on the follower, not the publisher.

use anyhow::Result;
use diaswarm_core::{Record, EPOCH_MS};
use diaswarm_keys::wire::{self, KeysArgs, LOG_ID};
use diaswarm_keys::{open_bounded_store, Rng, Vault};
use diaswarm_net::pool;
use diaswarm_net::replicate::KeysReplicator;
use diaswarm_net::swarm::{network_id, Swarm};
use futures_util::StreamExt;
use p2panda_core::{AnyOperation, Hash, SigningKey, Topic, VerifyingKey};
use p2panda_net::LogSync;
use p2panda_store::logs::{LogStore, LogStream};
use p2panda_store::operations::OperationStore;
use p2panda_store::topics::TopicStore;
use p2panda_store::{tx, SqliteError, SqliteStore};
use std::collections::BTreeMap;

const OFFSET: i64 = 12 * 3_600_000;
const RSS_CAP_KB: u64 = 8 * 1024 * 1024; // 8 GiB — stop before the host OOMs.

/// **THE FOLLOWER-SIDE WINDOW (step 2 prototype).** A thin wrapper over the real
/// disk `SqliteStore` that lies about ONE thing: the height it advertises for the
/// subject's SEGMENT log. The whole sync negotiation turns on `get_log_heights`
/// — `log_sync` builds its `Have` from it, and the remote sends only operations
/// ABOVE what we claim to hold ([`p2panda_sync::protocols::log_sync`], SendHave).
///
/// So a fresh carrier that advertises `Have[floor]` for the segment log pulls
/// only `(floor, tip]` — the last `tip - floor` ops — no matter how much history
/// the publisher (or a stale carrier) still serves. It is the follower-side lever
/// the publisher-prune (D35) cannot be: it bounds what WE REQUEST, not what the
/// source happens to keep. Mid-log ingest is safe because the sync never checks a
/// received op's backlink against our store (prune is a plain row delete; the
/// `keep`-path run below with a fresh carrier proves it).
///
/// The CONTROL log (keys/welcomes) is NEVER windowed — a reader needs all of it —
/// so the floor is applied to [`LOG_ID`] alone, and the real height is reported
/// for everything else. Received operations are written straight to `inner` by the
/// drain loop, so this wrapper only has to satisfy the sync's READ path
/// (`LogStore` + `TopicStore`); every other call delegates unchanged.
#[derive(Clone)]
struct WindowedStore {
    inner: SqliteStore,
    subject: VerifyingKey,
    floor: u32,
}

// SqliteStore implements these traits generically over several parameter sets,
// so every delegation is spelled out with the exact type arguments LogSync wants
// — a bare `self.inner.method(..)` is ambiguous (E0283).
impl LogStore<AnyOperation, VerifyingKey, u32, u32, Hash> for WindowedStore {
    type Error = SqliteError;

    async fn get_log_heights(
        &self,
        author: &VerifyingKey,
        logs: &[u32],
    ) -> Result<Option<BTreeMap<u32, u32>>, Self::Error> {
        let mut heights =
            <SqliteStore as LogStore<AnyOperation, VerifyingKey, u32, u32, Hash>>::get_log_heights(&self.inner, author, logs).await?.unwrap_or_default();
        // Only the subject we are windowing, and only its SEGMENT log.
        if *author == self.subject && logs.contains(&LOG_ID) {
            let real = heights.get(&LOG_ID).copied().unwrap_or(0);
            heights.insert(LOG_ID, real.max(self.floor));
        }
        Ok(if heights.is_empty() { None } else { Some(heights) })
    }

    async fn get_latest_entry_tx(
        &self,
        author: &VerifyingKey,
        log_id: &u32,
    ) -> Result<Option<AnyOperation>, Self::Error> {
        <SqliteStore as LogStore<AnyOperation, VerifyingKey, u32, u32, Hash>>::get_latest_entry_tx(&self.inner, author, log_id).await
    }

    async fn get_log_size(
        &self,
        author: &VerifyingKey,
        log_id: &u32,
        after: Option<u32>,
        until: Option<u32>,
    ) -> Result<Option<(u32, u32)>, Self::Error> {
        <SqliteStore as LogStore<AnyOperation, VerifyingKey, u32, u32, Hash>>::get_log_size(&self.inner, author, log_id, after, until).await
    }

    fn log_entries(
        &self,
        author: &VerifyingKey,
        log_id: &u32,
        after: Option<u32>,
        until: Option<u32>,
    ) -> Result<LogStream<AnyOperation, u32, Self::Error>, Self::Error> {
        <SqliteStore as LogStore<AnyOperation, VerifyingKey, u32, u32, Hash>>::log_entries(&self.inner, author, log_id, after, until)
    }

    async fn prune_entries_tx(
        &self,
        author: &VerifyingKey,
        log_id: &u32,
        until: &u32,
    ) -> Result<u64, Self::Error> {
        <SqliteStore as LogStore<AnyOperation, VerifyingKey, u32, u32, Hash>>::prune_entries_tx(&self.inner, author, log_id, until).await
    }
}

impl TopicStore<Topic, VerifyingKey, u32> for WindowedStore {
    type Error = SqliteError;

    async fn associate_tx(
        &self,
        topic: &Topic,
        author: &VerifyingKey,
        data_id: &u32,
    ) -> Result<bool, Self::Error> {
        <SqliteStore as TopicStore<Topic, VerifyingKey, u32>>::associate_tx(&self.inner, topic, author, data_id).await
    }

    async fn remove_tx(
        &self,
        topic: &Topic,
        author: &VerifyingKey,
        data_id: &u32,
    ) -> Result<bool, Self::Error> {
        <SqliteStore as TopicStore<Topic, VerifyingKey, u32>>::remove_tx(&self.inner, topic, author, data_id).await
    }

    async fn resolve(&self, topic: &Topic) -> Result<BTreeMap<VerifyingKey, Vec<u32>>, Self::Error> {
        <SqliteStore as TopicStore<Topic, VerifyingKey, u32>>::resolve(&self.inner, topic).await
    }

    async fn resolve_topics(
        &self,
        author: &VerifyingKey,
        data_id: &u32,
    ) -> Result<Vec<Topic>, Self::Error> {
        <SqliteStore as TopicStore<Topic, VerifyingKey, u32>>::resolve_topics(&self.inner, author, data_id).await
    }

    async fn topics(&self) -> Result<Vec<Topic>, Self::Error> {
        <SqliteStore as TopicStore<Topic, VerifyingKey, u32>>::topics(&self.inner).await
    }
}

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
    // keep: if >0, the publisher PRUNES its segment log to the last `keep` ops
    // before the carrier syncs — the D35 fix, minimal form (delete old, no flag).
    let keep: u32 = std::env::args().nth(3).and_then(|s| s.parse().ok()).unwrap_or(0);
    // window: if >0, the CARRIER advertises a windowed `Have` and syncs only the
    // last `window` segment ops, EVEN when the publisher serves the whole log
    // (step 2 — the follower-side lever). Independent of `keep`; the point is to
    // leave the publisher UNPRUNED and bound the ingest from the follower's side.
    let window: u32 = std::env::args().nth(4).and_then(|s| s.parse().ok()).unwrap_or(0);
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
    let mut total = held(&subject_store, &author).await;
    eprintln!("sealed {segs} segments; subject log holds {total} ops; rss={} MB", rss_kb() / 1024);

    // D35 PRUNE: delete everything but the last `keep` ops from the publisher's
    // shared segment log, so the sync has only a recent window to serve.
    if keep > 0 && total > keep {
        let until: u32 = total - keep;
        let pruned: u64 = tx!(subject_store, {
            <SqliteStore as LogStore<p2panda_core::AnyOperation, VerifyingKey, u32, u32, Hash>>::prune_entries_tx(
                &subject_store, &author, &LOG_ID, &until,
            )
            .await?
        });
        total = held(&subject_store, &author).await;
        eprintln!("pruned {pruned} ops (until seq {until}); publisher now serves {total} ops; rss={} MB", rss_kb() / 1024);
    }

    // The carrier under test: a fresh phone/Ayni joining and syncing the lot.
    let carrier_store = disk_store("carrierdb").await;
    let carrier = Swarm::join_network(tmp("car"), SigningKey::generate(), net).await.unwrap();
    let (ce, cg) = carrier.parts();
    let depth = pool::depth_for(2);
    let topic_bytes = pool::bucket_topic(depth, pool::bucket_of(&subject_hex, depth));

    // What the carrier will end up holding: the whole log, unless it windows.
    let target = if window > 0 { window.min(total) } else { total };
    // Keep the drain task + its handle alive for the run (dropping unsubscribes).
    let _drain = if window > 0 {
        let floor = total.saturating_sub(window);
        eprintln!(
            "carrier WINDOWED: floor={floor}, advertising Have[{floor}] on the segment log \
             — should pull only the last {target} of {total}"
        );
        let windowed = WindowedStore { inner: carrier_store.clone(), subject: author, floor };
        let sync = LogSync::<WindowedStore, u32, KeysArgs>::builder(windowed, ce, cg)
            .spawn()
            .await
            .unwrap();
        // Associate the subject's logs with the topic on the real store (the sync
        // reads these back via `resolve` to know whom to ask), then subscribe.
        let topic: Topic = Hash::from_bytes(topic_bytes).into();
        tx!(carrier_store, {
            for log_id in &diaswarm_keys::wire::LOG_IDS {
                carrier_store.associate_tx(&topic, &author, log_id).await?;
            }
        });
        let handle = sync.stream(topic, true).await.unwrap();
        let mut events = handle.subscribe().await.unwrap();
        let store = carrier_store.clone();
        let task = tokio::spawn(async move {
            while let Some(next) = events.next().await {
                if let Ok(p2panda_sync::FromSync {
                    event:
                        p2panda_sync::protocols::TopicLogSyncEvent::OperationReceived { operation, .. },
                    ..
                }) = next
                {
                    let log_id = KeysArgs::log_id(&operation.header.extensions);
                    // tx! uses `?` internally, so it needs a Result-returning
                    // context — this spawned task returns ().
                    let _ = async {
                        tx!(store, {
                            store
                                .insert_operation_tx(&operation.hash, &*operation, &log_id)
                                .await?;
                        });
                        Ok::<(), SqliteError>(())
                    }
                    .await;
                }
            }
        });
        // KEEP `sync` ALIVE: dropping the LogSync stops its background actor and
        // the session never runs (KeysReplicator keeps it in a field for exactly
        // this reason). Holding only the stream handle is not enough.
        Some((sync, handle, task))
    } else {
        let carrying = KeysReplicator::keys(carrier_store.clone(), ce, cg).await.unwrap();
        carrying.carry(topic_bytes, &subject_hex).await.unwrap();
        // Hold the replicator so its subscription stays live for the whole run.
        std::mem::forget(carrying);
        None
    };

    // The publisher serves the backlog (FULL log in window mode — that is the point).
    let publisher = Swarm::join_network(tmp("pub"), SigningKey::generate(), net).await.unwrap();
    let (pe, pg) = publisher.parts();
    let publishing = KeysReplicator::keys(subject_store.clone(), pe, pg).await.unwrap();
    publishing.carry(topic_bytes, &subject_hex).await.unwrap();

    eprintln!("carrier syncing toward {target} ops (publisher serves {total}); rss by the second (cap {} MB):", RSS_CAP_KB / 1024);
    println!("t_s,rss_kb,held,of");
    let start = std::time::Instant::now();
    let (mut last, mut flat) = (0u32, 0);
    loop {
        let h = held(&carrier_store, &author).await;
        let r = rss_kb();
        println!("{:.0},{},{},{}", start.elapsed().as_secs_f64(), r, h, target);
        if r > RSS_CAP_KB {
            eprintln!("RSS CAP hit at {} MB, held {h}/{target} — the ingest is unbounded", r / 1024);
            break;
        }
        if h >= target {
            eprintln!("CAUGHT UP at rss={} MB (held {h}/{target}; publisher served {total})", r / 1024);
            break;
        }
        flat = if h == last { flat + 1 } else { 0 };
        if flat > 60 {
            eprintln!("STALLED at held {h}/{target}, rss={} MB", r / 1024);
            break;
        }
        last = h;
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    }
    Ok(())
}
