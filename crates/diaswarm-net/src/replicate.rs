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
    /// Of those, the ones that arrived *pushed* rather than fetched.
    ///
    /// **THE NUMBER THAT WOULD HAVE CAUGHT THE MISSING SEND HALF ON DAY ONE.**
    /// `received` counts both phases and so was always healthy: catch-up syncs
    /// delivered everything, and live mode delivered nothing, and one total
    /// cannot tell those apart.
    ///
    /// See [`count_live`] for how the two are told apart, and for the wrong
    /// rule that shipped first and reported 1,052 live arrivals on a phone
    /// whose only peer had pushed exactly one.
    live: Arc<Mutex<usize>>,
    /// The last `received_live_operations` seen on each session, for [`count_live`].
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
            live: Arc::clone(&self.live),
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
            live: Arc::new(Mutex::new(0)),
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
        let live = Arc::clone(&self.live);
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
                            let pushed = count_live(
                                &mut live_seen.lock().unwrap(),
                                (topic_bytes, *remote.as_bytes(), session_id),
                                metrics.received_live_operations,
                            );
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
                                Ok(()) => {
                                    *received.lock().unwrap() += 1;
                                    // **COUNTED ONLY IF IT WAS ALSO KEPT**, so
                                    // `live` can never exceed `received`. The
                                    // first two versions of this counter both
                                    // reported numbers that were not merely
                                    // wrong but arithmetically impossible, and
                                    // an impossible number is one nobody can
                                    // reason from. Tying it to the same branch
                                    // makes the invariant structural rather
                                    // than something to remember.
                                    *live.lock().unwrap() += pushed;
                                }
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
                                live_seen
                                    .lock()
                                    .unwrap()
                                    .remove(&(topic_bytes, *remote.as_bytes(), session_id));
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

    /// How many of those were pushed to us in live mode rather than fetched.
    ///
    /// Zero while `received` climbs means catch-up is carrying everything and
    /// the push half is not working — the state this project was in for its
    /// whole life, unnoticed, because nothing counted the two separately.
    pub fn live_received(&self) -> usize {
        *self.live.lock().unwrap()
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

/// How many of this event's operations arrived pushed rather than fetched.
///
/// **THE OBVIOUS RULE IS WRONG, AND IT SHIPPED FIRST.** `Metrics` is cumulative
/// over a *session*, so `received_live_operations > 0` means "this session has
/// received a live operation at some point", not "this operation is live". A
/// session that takes one push and then re-syncs — which is ordinary, and which
/// the event log shows happening as `SyncFinished, LiveModeStarted,
/// SyncFinished, LiveModeStarted` against one peer — makes every catch-up
/// operation after that point look pushed.
///
/// Measured on a phone within minutes of shipping it: a follower reported 1,052
/// live arrivals out of 1,059 while the only peer that could have pushed to it
/// reported `pushed 1`. The true figure was one. A counter that exists to
/// detect a broken transport is worth nothing if it reports success by
/// accident, so this counts the *increase* instead, per session.
///
/// A drop means a new session reusing the id, so the new value is the count.
fn count_live(
    seen: &mut std::collections::HashMap<Session, u32>,
    session: Session,
    received_live: u32,
) -> usize {
    // **FAIL TOWARDS UNDERCOUNTING.** Sessions are forgotten when they end, so
    // this cap should never be reached; if it is, something is already wrong.
    // Refusing a new baseline reports fewer pushes than happened, which causes
    // an investigation. Clearing the map instead would report the next
    // session's whole running total as new — over-reporting, which HIDES a
    // broken transport. That is the mistake this counter has already made
    // twice, and it is the one that costs a day.
    if !seen.contains_key(&session) && seen.len() >= KEEP_SESSIONS {
        return 0;
    }
    let previous = seen.insert(session, received_live).unwrap_or(0);
    if received_live >= previous {
        (received_live - previous) as usize
    } else {
        // The counter went backwards, so this is a different session wearing a
        // recycled id: everything it reports is new.
        received_live as usize
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
    use super::{count_live, Session, KEEP_SESSIONS};
    use std::collections::HashMap;

    fn session(topic: u8, peer: u8, id: u64) -> Session {
        ([topic; 32], [peer; 32], id)
    }

    /// A LIVE COUNT THAT COUNTS CATCH-UP IS WORSE THAN NO LIVE COUNT.
    ///
    /// **THIS IS THE RULE THAT WAS WRONG ON A PHONE.** The first version asked
    /// `received_live_operations > 0`, which is a fact about the *session*, not
    /// about the operation in hand. A follower reported 1,052 live arrivals out
    /// of 1,059 while its only peer had pushed one — so the number that exists
    /// to prove the transport works was proving it by accident, which is the
    /// same failure shape as the defect it was added to detect.
    #[test]
    fn catch_up_after_a_push_is_not_a_push() {
        let mut seen = HashMap::new();
        let s = session(1, 1, 7);
        for _ in 0..1_000 {
            assert_eq!(count_live(&mut seen, s, 0), 0);
        }
        assert_eq!(count_live(&mut seen, s, 1), 1);
        // The session re-syncs and delivers fifty more by catch-up. The
        // cumulative metric still says 1, and none of these are live.
        for _ in 0..50 {
            assert_eq!(count_live(&mut seen, s, 1), 0, "catch-up counted as a push");
        }
        assert_eq!(count_live(&mut seen, s, 2), 1);
    }

    /// TWO TOPICS DO NOT RE-COUNT EACH OTHER'S TOTALS.
    ///
    /// **THE SECOND WRONG VERSION, AND THE PHONE SAID SO IN ARITHMETIC.** Keyed
    /// on `session_id` alone this reported 17,128 live arrivals out of 8,103
    /// operations received — not merely wrong but impossible. A replicator
    /// streams a topic per subject and each manager numbers its own sessions,
    /// so two topics run sessions numbered alike; interleaved, every switch
    /// reads as a session starting over and counts the whole running total
    /// again.
    #[test]
    fn sessions_numbered_alike_on_different_topics_are_different_sessions() {
        let mut seen = HashMap::new();
        let (a, b) = (session(1, 9, 4), session(2, 9, 4));
        assert_eq!(count_live(&mut seen, a, 1), 1);
        assert_eq!(count_live(&mut seen, b, 1), 1, "the second topic started at its own zero");
        // Interleaved, each only counts its own increase.
        for n in 2..20u32 {
            assert_eq!(count_live(&mut seen, a, n), 1);
            assert_eq!(count_live(&mut seen, b, n), 1);
        }
        let total: u32 = 19 + 19;
        assert_eq!(total, 38, "two topics, nineteen pushes each");
    }

    /// AND NEITHER DO TWO PEERS ON ONE TOPIC.
    ///
    /// Two phones and a laptop carrying the same subject is the ordinary case.
    #[test]
    fn peers_do_not_borrow_each_others_counts() {
        let mut seen = HashMap::new();
        let (a, b) = (session(3, 1, 2), session(3, 2, 2));
        assert_eq!(count_live(&mut seen, a, 1), 1);
        assert_eq!(count_live(&mut seen, b, 1), 1);
        assert_eq!(count_live(&mut seen, a, 2), 1);
        assert_eq!(count_live(&mut seen, b, 2), 1);
    }

    /// A RESTARTED SESSION DOES NOT LOSE ITS PUSHES.
    ///
    /// A counter that went backwards is a session starting over, not a negative
    /// number of operations. Saturating to zero would undercount silently for
    /// the life of the new session.
    #[test]
    fn a_session_id_that_starts_over_starts_over() {
        let mut seen = HashMap::new();
        let s = session(4, 4, 3);
        assert_eq!(count_live(&mut seen, s, 9), 9);
        assert_eq!(count_live(&mut seen, s, 2), 2, "a restarted session lost its pushes");
        assert_eq!(count_live(&mut seen, s, 3), 1);
    }

    /// THE BASELINE MAP IS BOUNDED, AND OVERFLOWS TOWARDS SILENCE.
    ///
    /// **THE DIRECTION MATTERS MORE THAN THE CAP.** Refusing new baselines
    /// reports fewer pushes than happened, which causes somebody to go and
    /// look. Clearing the map instead would report the next session's whole
    /// running total as new — over-reporting, which HIDES a broken transport.
    /// This counter has already over-reported twice; it must not be able to a
    /// third time.
    #[test]
    fn a_full_baseline_map_undercounts_rather_than_inventing() {
        let mut seen = HashMap::new();
        for i in 0..KEEP_SESSIONS as u64 {
            assert_eq!(count_live(&mut seen, session(0, 0, i), 1), 1);
        }
        assert_eq!(seen.len(), KEEP_SESSIONS);
        // A session beyond the cap is not counted, and does not evict anybody.
        assert_eq!(count_live(&mut seen, session(0, 0, 9_999), 500), 0);
        assert_eq!(seen.len(), KEEP_SESSIONS, "the cap evicted a live session's baseline");
        // And a session already known still counts, cap or no cap.
        assert_eq!(count_live(&mut seen, session(0, 0, 0), 2), 1);
    }

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
