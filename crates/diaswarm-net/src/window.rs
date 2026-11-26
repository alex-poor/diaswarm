//! The follower-side sync window: a read view over the real `SqliteStore` that
//! advertises a FLOORED segment-log height per subject.
//!
//! **WHY IT IS A VIEW, NOT A STORE.** `p2panda`'s log sync turns entirely on the
//! `Have` each side builds from `get_log_heights` (see `log_sync`, SendHave): a
//! peer sends only operations ABOVE the height you claim. So a follower that wants
//! only "the last N days" of a subject advertises a height at the start of that
//! window and receives just the tail — no matter how much history the publisher or
//! any carrier still serves. That is the follower-side lever publisher-pruning
//! cannot be: it bounds what WE REQUEST. Mid-log ingest is safe because the sync
//! never re-validates a received op's backlink against the local store.
//!
//! The window lives HERE, on the read path, and leaves writing alone: a
//! [`crate::replicate::Replicator`] keeps the real `SqliteStore` for its own
//! writes and routes only the sync's reads through this. The floor map is shared
//! and mutable, because a follower cannot know a subject's floor until that
//! subject's control log — hence its epoch index — has synced; it starts empty, so
//! until then (and for every publisher and carried stranger) this is pure
//! passthrough.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};

use diaswarm_keys::wire::LOG_ID;
use p2panda_core::{AnyOperation, Hash, Topic, VerifyingKey};
use p2panda_store::logs::{LogStore, LogStream};
use p2panda_store::topics::TopicStore;
use p2panda_store::{SqliteError, SqliteStore};

/// A windowing read view over a `SqliteStore`, shared by clone (the floor map is
/// behind an `Arc`, so every clone sees the same windows).
#[derive(Clone)]
pub struct WindowedStore {
    inner: SqliteStore,
    /// subject verifying key → the segment-log height to advertise. Only the
    /// SEGMENT log ([`LOG_ID`]) is windowed; the control log is always full.
    floors: Arc<Mutex<HashMap<VerifyingKey, u32>>>,
}

impl WindowedStore {
    /// A view that windows nothing until a floor is set — identical to the inner
    /// store for every read.
    pub fn passthrough(inner: SqliteStore) -> Self {
        Self { inner, floors: Arc::new(Mutex::new(HashMap::new())) }
    }

    /// Window `subject`'s segment log at `floor`: advertise `max(real, floor)` so a
    /// peer sends only operations above it. Idempotent; a later call replaces it.
    pub fn set_floor(&self, subject: VerifyingKey, floor: u32) {
        self.floors.lock().unwrap().insert(subject, floor);
    }

    /// Stop windowing `subject` (advertise its real height again).
    pub fn clear(&self, subject: &VerifyingKey) {
        self.floors.lock().unwrap().remove(subject);
    }

    fn floor_for(&self, author: &VerifyingKey) -> Option<u32> {
        self.floors.lock().unwrap().get(author).copied()
    }
}

// SqliteStore implements these traits generically over several parameter sets, so
// every delegation is spelled out with the exact type arguments — a bare
// `self.inner.method(..)` is ambiguous (E0283).
impl LogStore<AnyOperation, VerifyingKey, u32, u32, Hash> for WindowedStore {
    type Error = SqliteError;

    async fn get_log_heights(
        &self,
        author: &VerifyingKey,
        logs: &[u32],
    ) -> Result<Option<BTreeMap<u32, u32>>, Self::Error> {
        let mut heights =
            <SqliteStore as LogStore<AnyOperation, VerifyingKey, u32, u32, Hash>>::get_log_heights(
                &self.inner,
                author,
                logs,
            )
            .await?
            .unwrap_or_default();
        // Raise the segment-log height to the floor for this subject, if windowed.
        if logs.contains(&LOG_ID) {
            if let Some(floor) = self.floor_for(author) {
                let real = heights.get(&LOG_ID).copied().unwrap_or(0);
                heights.insert(LOG_ID, real.max(floor));
            }
        }
        Ok(if heights.is_empty() { None } else { Some(heights) })
    }

    async fn get_latest_entry_tx(
        &self,
        author: &VerifyingKey,
        log_id: &u32,
    ) -> Result<Option<AnyOperation>, Self::Error> {
        <SqliteStore as LogStore<AnyOperation, VerifyingKey, u32, u32, Hash>>::get_latest_entry_tx(
            &self.inner,
            author,
            log_id,
        )
        .await
    }

    async fn get_log_size(
        &self,
        author: &VerifyingKey,
        log_id: &u32,
        after: Option<u32>,
        until: Option<u32>,
    ) -> Result<Option<(u32, u32)>, Self::Error> {
        <SqliteStore as LogStore<AnyOperation, VerifyingKey, u32, u32, Hash>>::get_log_size(
            &self.inner,
            author,
            log_id,
            after,
            until,
        )
        .await
    }

    fn log_entries(
        &self,
        author: &VerifyingKey,
        log_id: &u32,
        after: Option<u32>,
        until: Option<u32>,
    ) -> Result<LogStream<AnyOperation, u32, Self::Error>, Self::Error> {
        <SqliteStore as LogStore<AnyOperation, VerifyingKey, u32, u32, Hash>>::log_entries(
            &self.inner,
            author,
            log_id,
            after,
            until,
        )
    }

    async fn prune_entries_tx(
        &self,
        author: &VerifyingKey,
        log_id: &u32,
        until: &u32,
    ) -> Result<u64, Self::Error> {
        <SqliteStore as LogStore<AnyOperation, VerifyingKey, u32, u32, Hash>>::prune_entries_tx(
            &self.inner,
            author,
            log_id,
            until,
        )
        .await
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
        <SqliteStore as TopicStore<Topic, VerifyingKey, u32>>::associate_tx(
            &self.inner,
            topic,
            author,
            data_id,
        )
        .await
    }

    async fn remove_tx(
        &self,
        topic: &Topic,
        author: &VerifyingKey,
        data_id: &u32,
    ) -> Result<bool, Self::Error> {
        <SqliteStore as TopicStore<Topic, VerifyingKey, u32>>::remove_tx(
            &self.inner,
            topic,
            author,
            data_id,
        )
        .await
    }

    async fn resolve(
        &self,
        topic: &Topic,
    ) -> Result<BTreeMap<VerifyingKey, Vec<u32>>, Self::Error> {
        <SqliteStore as TopicStore<Topic, VerifyingKey, u32>>::resolve(&self.inner, topic).await
    }

    async fn resolve_topics(
        &self,
        author: &VerifyingKey,
        data_id: &u32,
    ) -> Result<Vec<Topic>, Self::Error> {
        <SqliteStore as TopicStore<Topic, VerifyingKey, u32>>::resolve_topics(
            &self.inner,
            author,
            data_id,
        )
        .await
    }

    async fn topics(&self) -> Result<Vec<Topic>, Self::Error> {
        <SqliteStore as TopicStore<Topic, VerifyingKey, u32>>::topics(&self.inner).await
    }
}
