//! Moving subjects between peers with p2panda's log sync.
//!
//! WHAT THIS REPLACES. [`crate::wire`] is a hand-written request/response
//! protocol — `Have`, `Manifest`, `Grants`, `Segment`, `Wraps` — that a peer
//! drives by polling every two minutes. p2panda ships log sync: a peer says
//! which logs a topic covers, subscribes, and the library catches it up and
//! then keeps it live over gossip. Measured in `spike/p2panda-logsync`,
//! including that it carries operation bodies up to at least 4 MB, which is
//! what `diaswarm-spaces` puts record ciphertext in.
//!
//! HOW IT MEETS THE POOL, WHICH IS THE PART WORTH READING. Nothing in
//! [`crate::pool`] changes. A bucket topic already carries a gossip
//! announcement of *which* subjects live in it; log sync carries *what those
//! subjects contain*, over the same topic. So the division is:
//!
//!   * **gossip** answers "who is in this bucket" — a list of subject keys;
//!   * **log sync** answers "give me their operations", once a peer has
//!     associated a subject's log with the topic.
//!
//! A peer holding a bucket therefore associates every subject it hears about
//! there and gets the data without asking anyone for anything. That is the same
//! self-healing arithmetic the pool always had, with the transport handed to
//! the library.
//!
//! **A SUBJECT IS AN AUTHOR AND SOME LOGS.** `diaswarm-spaces` publishes
//! everything a subject ever seals into log 0 of the subject's own key, so
//! associating `(topic, subject_key, 0)` was the whole of "carry this person's
//! data". `diaswarm-keys` splits that in two — segments in log 0, grants in log
//! 1 — so this is parameterised over the logs a subject has and over the
//! extension type they carry, rather than hard-wired to one of each.
//!
//! **AND THAT IS WHY BOTH OF ITS LOGS CARRY ONE EXTENSION TYPE.**
//! `LogSync<S, L, E>` takes a single `E`, and two instances cannot share an
//! endpoint, so a vault with two differently-shaped logs has to make them one
//! shape: `diaswarm_keys::wire::KeysArgs` is that enum, and `log_of` below is
//! how an arriving operation says which of the two it belongs in.
//!
//! **RECEIVING IS NOT HOLDING.** Log sync hands an application the operations
//! it fetched and stops there — storing them is the application's decision,
//! which is right for a library and easy to miss. A first version of this
//! counted `OperationReceived` events and looked like it worked: sync started,
//! six operations and 2,602 bytes crossed, sync finished, live mode began, and
//! the carrier's store was empty. A peer that carries nothing while reporting
//! healthy sync is the worst shape a bug in this project can take.
//!
//! **AND PUBLISHING IS NOT SENDING** — the same mistake at the other end of the
//! wire, and it survived every build this project shipped. Log sync catches a
//! peer up and then switches to live mode, where new operations are *pushed*
//! over gossip by `SyncHandle::publish`. Nothing here ever called it: `stream`
//! moved the handle into its spawned task, where it kept the subscription alive
//! and was unreachable for ever after. So a subject wrote operations to its
//! store — which is not on the network — and they reached a follower whenever
//! the next catch-up sync happened to run.
//!
//! It hid behind healthy-looking totals: operations arrived, sessions
//! succeeded, and staleness got blamed on doze, on Wi-Fi locks, on the relay
//! and on a one-shot subscription in turn. The thing that said it plainly was
//! `received_live_operations: 0` in all sixty-nine live-mode sessions across
//! two phones. [`Replicator::broadcast`] is the missing half, and
//! [`Replicator::live_received`] is the counter that makes its absence visible
//! rather than inferable.
//!
//! WHAT THIS CANNOT DO. Start a sync on demand: `SyncHandle::initiate_session`
//! is `#[cfg(test)]` upstream. A peer subscribes and waits for discovery to
//! find someone who shares the topic. For the pool that is right — nobody is
//! dialled, everyone is found — but a follower that has just scanned an invite
//! gets its data when discovery gets round to it rather than immediately.

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use futures_util::StreamExt;
use p2panda_core::{Hash, Topic, VerifyingKey};
use p2panda_net::sync::SyncHandle;
use p2panda_net::{Endpoint, Gossip, LogSync};
use p2panda_store::operations::OperationStore;
use p2panda_store::topics::TopicStore;
use p2panda_store::{SqliteStore, tx};
use p2panda_sync::FromSync;
use p2panda_sync::protocols::TopicLogSyncEvent;

use p2panda_core::Extensions;

use diaswarm_spaces::Conditions;

/// The extensions `diaswarm-spaces` operations carry.
pub type SpacesArgs = diaswarm_spaces::SpacesArgs<Conditions>;

/// The extensions `diaswarm-keys` operations carry, across both its logs.
pub type KeysArgs = diaswarm_keys::wire::KeysArgs;

/// The live stream handle for one topic, and what it carries.
type Stream<A> = SyncHandle<p2panda_core::Operation<A>, TopicLogSyncEvent<A>>;

/// Every topic's live stream handle, shared by every clone of a replicator.
type Handles<A> = Arc<Mutex<std::collections::HashMap<[u8; 32], Stream<A>>>>;

/// One sync session: which topic, which peer, which session of theirs.
///
/// **ALL THREE, BECAUSE `session_id` ALONE IS NOT UNIQUE.** A replicator streams
/// a topic per subject and each has its own manager handing out session ids, so
/// two topics run sessions numbered alike. Keyed on the id alone, their
/// cumulative counters interleave and each switch reads as a session starting
/// over — which counts the whole running total again. Measured on a phone: a
/// follower reported 17,128 live arrivals out of 8,103 operations received,
/// which is not merely wrong but impossible.
type Session = ([u8; 32], [u8; 32], u64);

/// Replication for a `diaswarm-spaces` peer.
pub type SpacesReplicator = Replicator<SpacesArgs>;

/// Replication for a `diaswarm-keys` peer: segments and grants, one session.
pub type KeysReplicator = Replicator<KeysArgs>;

/// Replication for one peer: its store, and the topics it is carrying.
pub struct Replicator<A: Extensions + Send + 'static> {
    store: SqliteStore,
    sync: LogSync<SqliteStore, u32, A>,
    /// Every log a subject publishes to. Associated together, because a
    /// follower that gets segments and not grants cannot open them, and one
    /// that gets grants and not segments has nothing to open.
    logs: Vec<u32>,
    /// Which log an arriving operation belongs in.
    ///
    /// A function rather than a constant because `diaswarm-keys` has two, and
    /// storing an operation in the wrong one is not recoverable: the header is
    /// already signed and the sequence numbers of the other log are already
    /// claimed.
    log_of: fn(&A) -> u32,
    /// Topics already streamed, so a repeated pass does not subscribe twice.
    streaming: Arc<Mutex<HashSet<[u8; 32]>>>,
    /// The task consuming each topic's events, so a dead one can be replaced.
    ///
    /// **BECAUSE SUBSCRIBING ONCE IS SUBSCRIBING FOREVER, AND THAT IS THE BUG.**
    /// `stream` catches up and then relies on gossip to push. When the gossip
    /// link dies — the other phone changed network — the stream goes quiet and
    /// nothing notices: `carry` returns early on every later pass because the
    /// topic is already in `streaming`, and the app reports "carrying 2 logs"
    /// from a cached set while no bytes move. Measured: eleven minutes, then a
    /// recurrence five minutes after a restart.
    ///
    /// Aborting the task drops the handle it holds, which unsubscribes, which
    /// is what makes re-streaming possible at all.
    tasks: Arc<Mutex<std::collections::HashMap<[u8; 32], tokio::task::JoinHandle<()>>>>,
    /// The live stream handle for each topic, kept so new operations can be
    /// *pushed* as well as received.
    ///
    /// **THE SENDING HALF OF LIVE MODE, WHICH WAS NEVER WIRED.** p2panda's
    /// contract is that after catch-up "nodes switch to live-mode to directly
    /// push new messages to the network using a gossip protocol", and the way
    /// to push is `SyncHandle::publish`. Nothing in this project ever called
    /// it: operations were written to the store and the store is invisible to
    /// gossip. Measured on two phones — 69 `LiveModeStarted` and
    /// `received_live_operations: 0` every single time, for the life of the
    /// app. Every byte a follower ever received came from a catch-up sync.
    ///
    /// Holding the handle here also keeps the subscription alive, which is what
    /// the spawned task used to do by owning it.
    handles: Handles<A>,
    /// When an operation last arrived on any topic.
    ///
    /// One clock rather than per topic: the two logs of one subject are
    /// associated together and go quiet together, and a follower with several
    /// subjects would rather re-stream one topic too many than miss the one
    /// that matters.
    last_event: Arc<Mutex<std::time::Instant>>,
    /// `(topic, subject)` pairs already associated.
    associated: Arc<Mutex<HashSet<([u8; 32], String)>>>,
    /// Operations seen arriving, for tests and for reporting progress.
    received: Arc<Mutex<usize>>,
    /// Live arrivals from sessions that have since ended.
    ///
    /// **NOT DERIVED ANY MORE — THIS IS p2panda's OWN NUMBER.** Four versions
    /// tried to reconstruct "was *this* operation pushed?" from
    /// `Metrics::received_live_operations`, which is cumulative over a session,
    /// and all four over-reported: 1,052 live of 1,059; then 17,128 of 8,103;
    /// then 5,245 of 3,322; then 4,872 of 4,890 against a publisher that had
    /// pushed four times. The last failed even as a per-event boolean, because
    /// the counter advances two at a time per event this stream observes —
    /// measured on a laptop as `n=2,4,6,8,…`.
    ///
    /// So nothing is reconstructed. `live_seen` holds each open session's own
    /// counter exactly as p2panda last reported it, this holds the total from
    /// sessions that have ended, and [`Replicator::live_received`] adds them.
    /// There is no rule left to get wrong.
    live_retired: Arc<Mutex<usize>>,
    /// Each open session's `received_live_operations`, as p2panda last said it.
    live_seen: Arc<Mutex<std::collections::HashMap<Session, u32>>>,
    /// Every sync event, in order.
    ///
    /// Kept because "nothing replicated" has several very different causes —
    /// no peer found, a session that started and failed, a session that
    /// finished having transferred nothing — and they are indistinguishable
    /// from a count of zero.
    events: Arc<Mutex<Vec<String>>>,
}

/// **CLONEABLE, BECAUSE THE PUBLISHER IS NOT THE POOL.** Every field is either
/// already shared (`Arc`), a cheap handle (`SqliteStore`, `LogSync`) or plain
/// data, so a clone is the same replicator seen from somewhere else — the same
/// stream handles, the same tasks, the same counters. It exists so the vault
/// that seals a segment can push it without the pool handle being threaded
/// through every JNI call that touches a vault.
impl<A: Extensions + Send + 'static> Clone for Replicator<A> {
    fn clone(&self) -> Self {
        Replicator {
            store: self.store.clone(),
            sync: self.sync.clone(),
            logs: self.logs.clone(),
            log_of: self.log_of,
            streaming: Arc::clone(&self.streaming),
            tasks: Arc::clone(&self.tasks),
            handles: Arc::clone(&self.handles),
            last_event: Arc::clone(&self.last_event),
            associated: Arc::clone(&self.associated),
            received: Arc::clone(&self.received),
            live_retired: Arc::clone(&self.live_retired),
            live_seen: Arc::clone(&self.live_seen),
            events: Arc::clone(&self.events),
        }
    }
}

impl SpacesReplicator {
    /// Start replication for a `diaswarm-spaces` peer: one log, one shape.
    pub async fn spaces(store: SqliteStore, endpoint: Endpoint, gossip: Gossip) -> Result<Self> {
        Self::start(store, endpoint, gossip, &[diaswarm_spaces::LOG_ID], |_| {
            diaswarm_spaces::LOG_ID
        })
        .await
    }
}

impl KeysReplicator {
    /// Start replication for a `diaswarm-keys` peer: segments and grants.
    pub async fn keys(store: SqliteStore, endpoint: Endpoint, gossip: Gossip) -> Result<Self> {
        Self::start(
            store,
            endpoint,
            gossip,
            &diaswarm_keys::wire::LOG_IDS,
            diaswarm_keys::wire::KeysArgs::log_id,
        )
        .await
    }
}

impl<A: Extensions + Send + Sync + 'static> Replicator<A> {
    /// Start replication on a peer's existing endpoint and gossip.
    ///
    /// Takes them rather than making them: see [`crate::swarm::Swarm::parts`].
    pub async fn start(
        store: SqliteStore,
        endpoint: Endpoint,
        gossip: Gossip,
        logs: &[u32],
        log_of: fn(&A) -> u32,
    ) -> Result<Self> {
        let sync = LogSync::<SqliteStore, u32, A>::builder(store.clone(), endpoint, gossip)
            .spawn()
            .await
            .context("spawning log sync")?;
        Ok(Replicator {
            store,
            sync,
            logs: logs.to_vec(),
            log_of,
            streaming: Arc::new(Mutex::new(HashSet::new())),
            tasks: Arc::new(Mutex::new(std::collections::HashMap::new())),
            handles: Arc::new(Mutex::new(std::collections::HashMap::new())),
            last_event: Arc::new(Mutex::new(std::time::Instant::now())),
            associated: Arc::new(Mutex::new(HashSet::new())),
            received: Arc::new(Mutex::new(0)),
            live_retired: Arc::new(Mutex::new(0)),
            live_seen: Arc::new(Mutex::new(std::collections::HashMap::new())),
            events: Arc::new(Mutex::new(Vec::new())),
        })
    }

    /// Carry this subject's data on this topic.
    ///
    /// Idempotent, because a pool pass calls it for everything it hears about
    /// and most of that is already known.
    pub async fn carry(&self, topic_bytes: [u8; 32], subject: &str) -> Result<()> {
        let key = subject_key(subject)?;
        let pair = (topic_bytes, subject.to_string());
        if self.associated.lock().unwrap().contains(&pair) {
            return Ok(());
        }

        let topic: Topic = Hash::from_bytes(topic_bytes).into();
        let store = &self.store;
        // IN A TRANSACTION. Every write on `SqliteStore` runs against a permit
        // from `begin`, including the methods that do not end in `_tx` and look
        // self-contained. Without it this fails with "tried to interact with
        // inexistant transaction", which reads like a corrupt store.
        let logs = self.logs.clone();
        let out: Result<(), p2panda_store::SqliteError> = async {
            tx!(store, {
                for log_id in &logs {
                    store.associate(&topic, &key, log_id).await?;
                }
            });
            Ok(())
        }
        .await;
        out.context("associating a subject's log with a topic")?;

        self.associated.lock().unwrap().insert(pair);
        self.stream(topic_bytes).await
    }

    /// Subscribe to a topic, once, and keep collecting from it.
    async fn stream(&self, topic_bytes: [u8; 32]) -> Result<()> {
        if !self.streaming.lock().unwrap().insert(topic_bytes) {
            return Ok(());
        }
        let topic: Topic = Hash::from_bytes(topic_bytes).into();

        // Live mode: catch up first, then keep receiving over gossip. This is
        // what removes the two-minute follower poll — new data is pushed rather
        // than asked for.
        let handle = self.sync.stream(topic, true).await.context("streaming a topic")?;
        let mut events = handle.subscribe().await.context("subscribing to a topic")?;
        let received = Arc::clone(&self.received);
        let live_retired = Arc::clone(&self.live_retired);
        let live_seen = Arc::clone(&self.live_seen);
        let log = Arc::clone(&self.events);
        let store = self.store.clone();
        let log_of = self.log_of;

        let last_event = Arc::clone(&self.last_event);
        // **KEPT, NOT MOVED INTO THE TASK.** Dropping the handle unsubscribes,
        // so it has to outlive this function either way — but it used to go
        // into the spawned task as `let _keep = handle`, where nothing could
        // ever reach it again. That is what made this peer receive-only:
        // `publish` lives on the handle, and the handle was buried.
        self.handles.lock().unwrap().insert(topic_bytes, handle);
        let task = tokio::spawn(async move {
            while let Some(next) = events.next().await {
                match next {
                    Ok(FromSync { event, remote, session_id, .. }) => match event {
                        TopicLogSyncEvent::OperationReceived { operation, metrics } => {
                            *last_event.lock().unwrap() = std::time::Instant::now();
                            // RECORDED, NOT JUDGED. Whatever p2panda says this
                            // session has received live is what gets reported.
                            let session = (topic_bytes, *remote.as_bytes(), session_id);
                            let n = metrics.received_live_operations;
                            let grew = {
                                let mut seen = live_seen.lock().unwrap();
                                // Bounded: sessions are dropped when they end, so
                                // reaching the cap means an end event never came.
                                if seen.contains_key(&session) || seen.len() < KEEP_SESSIONS {
                                    seen.insert(session, n) != Some(n)
                                } else {
                                    false
                                }
                            };
                            if grew && n > 0 {
                                // **NAMED, BECAUSE A TOTAL CANNOT BE CHECKED
                                // AGAINST ANYTHING.** A phone reported 4,872
                                // pushed arrivals while its only publisher had
                                // pushed four times, and nothing could say which
                                // peer they came from. A publisher's own count is
                                // this number's only external check, and it is
                                // per peer.
                                remember(
                                    &log,
                                    format!("live op from {} n={n}", &remote.to_hex()[..8]),
                                );
                            }
                            // STORED, OR THIS PEER CARRIES NOTHING.
                            //
                            // And stored in the log its own header says it
                            // belongs in — not a constant. A segment filed
                            // among grants would take a sequence number that
                            // the real grant at that height already holds.
                            let log_id = log_of(&operation.header.extensions);
                            let out: Result<(), p2panda_store::SqliteError> = async {
                                tx!(store, {
                                    store
                                        .insert_operation(
                                            &operation.hash,
                                            &*operation,
                                            &log_id,
                                        )
                                        .await?
                                });
                                Ok(())
                            }
                            .await;
                            match out {
                                Ok(()) => *received.lock().unwrap() += 1,
                                Err(e) => {
                                    remember(&log, format!("could not store: {e}"))
                                }
                            }
                        }
                        other => {
                            // A SESSION THAT HAS ENDED KEEPS NO BASELINE. This
                            // is what bounds `live_seen` in normal running: an
                            // id is only interesting while its session is live.
                            if matches!(
                                other,
                                TopicLogSyncEvent::SessionFinished { .. }
                                    | TopicLogSyncEvent::Failed { .. }
                            ) {
                                // ITS COUNT IS KEPT, NOT DISCARDED. Dropping the
                                // entry is what bounds the map; adding it to the
                                // retired total is what stops the reported figure
                                // going backwards when a session closes.
                                if let Some(n) = live_seen
                                    .lock()
                                    .unwrap()
                                    .remove(&(topic_bytes, *remote.as_bytes(), session_id))
                                {
                                    *live_retired.lock().unwrap() += n as usize;
                                }
                            }
                            remember(&log, format!("{other:?} from {}", &remote.to_hex()[..8]))
                        }
                    },
                    Err(e) => remember(&log, format!("error: {e}")),
                }
            }
        });
        self.tasks.lock().unwrap().insert(topic_bytes, task);
        Ok(())
    }

    /// Re-subscribe to every topic if nothing has arrived for a while.
    ///
    /// **THE ONE-SHOT SUBSCRIPTION IS THE DEFECT.** `stream` catches up once and
    /// then waits for gossip to push. When that link dies — the other phone
    /// moved network — the stream goes quiet and nothing re-establishes it:
    /// `carry` returns early for ever because the topic is already in
    /// `streaming`, and the app goes on reporting that it carries two logs
    /// while no bytes move. Found on hardware as a follower eleven minutes
    /// stale with every internal signal claiming health, and again five minutes
    /// after a restart appeared to fix it.
    ///
    /// Aborting the task drops the sync handle, which unsubscribes; clearing
    /// `streaming` is what lets `stream` do its work a second time. The catch-up
    /// on re-subscribe is what actually recovers the gap.
    ///
    /// Returns how many topics were re-streamed, so a caller can say so rather
    /// than guess.
    ///
    /// **UNCONDITIONAL, AND THE CONDITION IT USED TO CARRY WAS WRONG.** It took
    /// a `quiet_for` and compared it against `last_event` — the time since ANY
    /// operation arrived. Measured on a phone: during a stall the freshness of
    /// the newest *record* reached 1025 seconds while operations kept trickling
    /// in from catch-up syncs, so `last_event` stayed recent and this returned
    /// 0 every time it was asked. It never fired once.
    ///
    /// The caller knows the thing that matters — how old the newest record is —
    /// and the caller is the only one who can know, because this has no idea
    /// how often the subject publishes. So the decision belongs there and this
    /// just does the work.
    pub async fn restream(&self) -> Result<usize> {
        let topics: Vec<[u8; 32]> = self.streaming.lock().unwrap().iter().copied().collect();
        for topic in &topics {
            if let Some(task) = self.tasks.lock().unwrap().remove(topic) {
                task.abort();
            }
            // Dropping the handle is what unsubscribes; aborting the task only
            // stops reading. Since the handle moved out of the task and into
            // `handles`, this is now the line that does it.
            self.handles.lock().unwrap().remove(topic);
            self.streaming.lock().unwrap().remove(topic);
        }
        for topic in &topics {
            self.stream(*topic).await?;
        }
        Ok(topics.len())
    }

    /// Push a freshly published operation to everyone listening to `subject`.
    ///
    /// **THIS IS THE HALF OF LIVE MODE THAT WAS MISSING FOR THE WHOLE LIFE OF
    /// THE PROJECT.** Writing an operation to the store makes it available to
    /// the *next* catch-up sync and to nothing else — the store is not on the
    /// network. p2panda's contract is that after catch-up "nodes switch to
    /// live-mode to directly push new messages to the network using a gossip
    /// protocol", and the pushing is `SyncHandle::publish`. Nothing here ever
    /// called it. Measured on two phones: 69 `LiveModeStarted` events against a
    /// peer that synced perfectly, and `received_live_operations: 0` every
    /// single time. Every byte a follower ever got came from catch-up, which is
    /// why freshness tracked the sync interval and never the publish.
    ///
    /// Addressed by subject rather than by topic because the topic is a pool
    /// bucket whose depth moves with the member count, and the caller that
    /// seals a segment has no business knowing that. `carry` already recorded
    /// which topic each subject went on; this reads it back.
    ///
    /// Returns how many topics it went out on — 0 means nobody is carrying this
    /// subject yet, which is a real answer and not an error.
    pub fn broadcast(&self, subject: &str, operation: p2panda_core::Operation<A>) -> usize
    where
        A: Clone,
    {
        let topics: Vec<[u8; 32]> = self
            .associated
            .lock()
            .unwrap()
            .iter()
            .filter(|(_, s)| s == subject)
            .map(|(t, _)| *t)
            .collect();

        let handles = self.handles.lock().unwrap();
        let mut sent = 0;
        for topic in topics {
            let Some(handle) = handles.get(&topic) else { continue };
            // A failure here means the topic's actor is gone, which `restream`
            // is the cure for. The operation is already in the store, so the
            // next catch-up still carries it: this is a lost push, not lost
            // data, and it must not fail the publish that produced it.
            match handle.publish(operation.clone()) {
                Ok(()) => sent += 1,
                Err(e) => remember(&self.events, format!("could not push live: {e}")),
            }
        }
        sent
    }

    /// How many operations have arrived from other peers.
    pub fn received(&self) -> usize {
        *self.received.lock().unwrap()
    }

    /// How many operations peers have pushed to us in live mode.
    ///
    /// **p2panda's OWN COUNT, SUMMED OVER SESSIONS — NOT A RECONSTRUCTION.**
    /// Zero while [`Replicator::received`] climbs means catch-up is carrying
    /// everything and the push half is not working, which is the state this
    /// project shipped in unnoticed for its whole life.
    ///
    /// It counts per session, so an operation delivered live on two sessions
    /// counts twice. That is what "live operations received" means here, and it
    /// beats the alternative: four earlier versions tried to turn these counters
    /// into a per-operation answer and every one over-reported. The
    /// `live op from …` lines in [`Replicator::events`] name the peer, so the
    /// figure can be checked against a publisher's own count.
    pub fn live_received(&self) -> usize {
        let open: usize = self.live_seen.lock().unwrap().values().map(|n| *n as usize).sum();
        *self.live_retired.lock().unwrap() + open
    }

    /// Every non-operation sync event so far.
    pub fn events(&self) -> Vec<String> {
        self.events.lock().unwrap().clone()
    }
}

/// How many sync events to keep.
///
/// **BECAUSE THIS RAN UNBOUNDED ON A PHONE THAT DRIVES AN INSULIN PUMP.** Every
/// non-operation event was appended for ever, and they are not small: a single
/// `SyncFinished` Debug-formats its whole `Metrics` struct at around 300 bytes.
/// A phone syncing every couple of minutes produces several per session, which
/// is megabytes a day of strings nothing ever reads — `keysSyncEvents` asks for
/// a tail of six. The publishing app has been killed for heap before, and when
/// it is killed the loop stops.
///
/// 256 is far more than any diagnostic has wanted and small enough to be free.
const KEEP_EVENTS: usize = 256;

/// How many in-flight sessions to keep a live-counting baseline for.
///
/// Bounded because this runs on a phone that drives an insulin pump. Entries
/// are removed when a session ends, so this is a backstop against a session
/// whose end event never arrives, not a working limit.
const KEEP_SESSIONS: usize = 256;

/// Record a sync event, forgetting the oldest once there are too many.
fn remember(log: &Mutex<Vec<String>>, line: String) {
    let mut log = log.lock().unwrap();
    log.push(line);
    if log.len() > KEEP_EVENTS {
        let excess = log.len() - KEEP_EVENTS;
        log.drain(..excess);
    }
}

/// A subject is named by its hex public key; log sync wants the key itself.
fn subject_key(subject: &str) -> Result<VerifyingKey> {
    let bytes = (0..subject.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&subject[i..i + 2], 16))
        .collect::<Result<Vec<u8>, _>>()
        .context("a subject is hex")?;
    let bytes: [u8; 32] = bytes.try_into().map_err(|_| anyhow::anyhow!("a subject is 32 bytes"))?;
    VerifyingKey::from_bytes(&bytes).context("a subject is a public key")
}

#[cfg(test)]
mod tests {
    /// THE EVENT LOG DOES NOT GROW FOR EVER.
    ///
    /// **IT DID, ON A PHONE THAT DRIVES AN INSULIN PUMP.** Every sync event was
    /// appended and none were ever dropped; a `SyncFinished` Debug-formats its
    /// whole `Metrics` at about 300 bytes, several per session, a session every
    /// couple of minutes — megabytes a day of strings that nothing reads, since
    /// the only consumer asks for a tail of six. That app has been killed for
    /// heap before, and when it is killed the loop stops.
    #[test]
    fn the_event_log_forgets_the_oldest_rather_than_growing() {
        let log = std::sync::Mutex::new(Vec::new());
        for i in 0..super::KEEP_EVENTS * 3 {
            super::remember(&log, format!("event {i}"));
        }
        let kept = log.lock().unwrap();
        assert_eq!(kept.len(), super::KEEP_EVENTS, "the log grew past its cap");
        // AND IT KEEPS THE NEWEST. Forgetting the recent ones would be worse
        // than growing: the tail is the only part a diagnostic ever asks for.
        assert_eq!(kept.last().unwrap(), &format!("event {}", super::KEEP_EVENTS * 3 - 1));
        assert_eq!(kept.first().unwrap(), &format!("event {}", super::KEEP_EVENTS * 2));
    }
}
