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
    /// `(topic, subject)` pairs already associated.
    associated: Arc<Mutex<HashSet<([u8; 32], String)>>>,
    /// Operations seen arriving, for tests and for reporting progress.
    received: Arc<Mutex<usize>>,
    /// Every sync event, in order.
    ///
    /// Kept because "nothing replicated" has several very different causes —
    /// no peer found, a session that started and failed, a session that
    /// finished having transferred nothing — and they are indistinguishable
    /// from a count of zero.
    events: Arc<Mutex<Vec<String>>>,
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
            associated: Arc::new(Mutex::new(HashSet::new())),
            received: Arc::new(Mutex::new(0)),
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
        let log = Arc::clone(&self.events);
        let store = self.store.clone();
        let log_of = self.log_of;

        // The handle has to outlive this function: dropping it unsubscribes.
        tokio::spawn(async move {
            let _keep = handle;
            while let Some(next) = events.next().await {
                match next {
                    Ok(FromSync { event, remote, .. }) => match event {
                        TopicLogSyncEvent::OperationReceived { operation, .. } => {
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
                                    log.lock().unwrap().push(format!("could not store: {e}"))
                                }
                            }
                        }
                        other => log
                            .lock()
                            .unwrap()
                            .push(format!("{other:?} from {}", &remote.to_hex()[..8])),
                    },
                    Err(e) => log.lock().unwrap().push(format!("error: {e}")),
                }
            }
        });
        Ok(())
    }

    /// How many operations have arrived from other peers.
    pub fn received(&self) -> usize {
        *self.received.lock().unwrap()
    }

    /// Every non-operation sync event so far.
    pub fn events(&self) -> Vec<String> {
        self.events.lock().unwrap().clone()
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
