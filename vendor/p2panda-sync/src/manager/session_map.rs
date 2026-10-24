// SPDX-License-Identifier: MIT OR Apache-2.0

//! Maintain mapping of generic topics to session ids and of session ids to a channel sender.
use std::collections::{HashMap, HashSet};
use std::hash::Hash;

/// Map of generic topics to session ids and of session ids to a channel sender.
#[derive(Clone, Debug)]
pub struct SessionTopicMap<T, TX> {
    pub(crate) session_tx_map: HashMap<u64, TX>,
    pub(crate) session_topic_map: HashMap<u64, T>,
    pub(crate) topic_session_map: HashMap<T, HashSet<u64>>,
}

impl<T, TX> Default for SessionTopicMap<T, TX> {
    fn default() -> Self {
        Self {
            session_tx_map: Default::default(),
            session_topic_map: Default::default(),
            topic_session_map: Default::default(),
        }
    }
}

impl<T, TX> SessionTopicMap<T, TX>
where
    T: Clone + Hash + Eq,
{
    /// Insert a session id with their topic and tx channel.
    pub fn insert_with_topic(&mut self, session_id: u64, topic: T, tx: TX) {
        self.session_topic_map.insert(session_id, topic.clone());
        self.topic_session_map
            .entry(topic.clone())
            .and_modify(|sessions| {
                sessions.insert(session_id);
            })
            .or_insert(HashSet::from_iter([session_id]));
        self.session_tx_map.insert(session_id, tx);
    }

    /// Drop a session from all mappings.
    ///
    /// Returns true if the session existed and was dropped, otherwise returns false when the
    /// session was known
    pub fn drop(&mut self, session_id: u64) -> bool {
        let Some(topic) = self.session_topic_map.remove(&session_id) else {
            return false;
        };
        // Remove the session from its topic bucket, and the bucket itself once
        // it empties — otherwise an empty `HashSet` lingers for every topic
        // whose last session ended (bounded by distinct-topic count, but there
        // is no reason to keep it).
        if let Some(sessions) = self.topic_session_map.get_mut(&topic) {
            sessions.remove(&session_id);
            if sessions.is_empty() {
                self.topic_session_map.remove(&topic);
            }
        }
        self.session_tx_map.remove(&session_id);
        true
    }

    /// Drop every session whose sender no longer passes `keep`; returns how many.
    ///
    /// **WHY — THE MANAGER'S OWN COPY IS INSERT-ONLY OTHERWISE.** `session()`
    /// inserts a session here, but only the event stream's *clone* of this map
    /// is reaped (by `drop_session`, reactively, when a live-forward hits a dead
    /// channel). The manager's original is never touched, so across gossip churn
    /// — a new session per NeighbourUp — it grows one `(session_id → topic,
    /// sender)` triple per session that ever ran, for the life of the process.
    /// The entries are small (the heavy broadcast rings live in
    /// `from_session_tx`, which *is* reaped), so this is a slow drift rather than
    /// the per-session ring leak, but it is still unbounded.
    ///
    /// A futures mpsc `Sender::is_closed()` is true once the session's receiver
    /// is gone — i.e. the session ended — so the manager prunes the closed ones
    /// on each `session()`, the same cadence at which this map grows and exactly
    /// how `manager_tx` is already pruned. Dropping goes through [`drop`] so the
    /// topic bucket stays coherent.
    pub fn retain<F>(&mut self, mut keep: F) -> usize
    where
        F: FnMut(&TX) -> bool,
    {
        let dead: Vec<u64> = self
            .session_tx_map
            .iter()
            .filter_map(|(id, tx)| (!keep(tx)).then_some(*id))
            .collect();
        for id in &dead {
            self.drop(*id);
        }
        dead.len()
    }

    /// Get the topic for a session id.
    ///
    /// Returns None of the session id was not known.
    pub fn topic(&self, session_id: u64) -> Option<&T> {
        self.session_topic_map.get(&session_id)
    }

    /// Get ids for all sessions associated with the given topic.
    pub fn sessions(&self, topic: &T) -> HashSet<u64> {
        self.topic_session_map
            .get(topic)
            .cloned()
            .unwrap_or_default()
    }

    /// Get a reference to a session sender.
    pub fn sender(&self, session_id: u64) -> Option<&TX> {
        self.session_tx_map.get(&session_id)
    }

    /// Get a mutable reference to a session sender.
    pub fn sender_mut(&mut self, session_id: u64) -> Option<&mut TX> {
        self.session_tx_map.get_mut(&session_id)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use futures_channel::mpsc;
    use p2panda_core::Topic;

    use crate::manager::SessionTopicMap;

    const SESSION1: u64 = 1;
    const SESSION2: u64 = 2;
    const SESSION3: u64 = 3;

    #[test]
    fn default_is_empty() {
        let map: SessionTopicMap<Topic, ()> = SessionTopicMap::default();
        assert!(map.session_tx_map.is_empty());
        assert!(map.session_topic_map.is_empty());
        assert!(map.topic_session_map.is_empty());
    }

    #[test]
    fn insert_with_topic() {
        let (tx, _rx) = mpsc::channel::<()>(128);
        let mut map = SessionTopicMap::default();
        let topic_a = Topic::random();

        map.insert_with_topic(SESSION1, topic_a, tx.clone());

        // Check session→topic mapping
        assert_eq!(map.topic(SESSION1), Some(&topic_a));

        // Check topic→session mapping
        assert_eq!(map.sessions(&topic_a), HashSet::from_iter([SESSION1]));

        // Channel should be retrievable
        assert!(map.sender(SESSION1).is_some());
    }

    #[test]
    fn drop_session() {
        let (tx, _rx) = mpsc::channel::<()>(128);
        let mut map = SessionTopicMap::default();
        let topic_a = Topic::random();
        let topic_b = Topic::random();

        map.insert_with_topic(SESSION1, topic_a, tx.clone());
        map.insert_with_topic(SESSION2, topic_a, tx.clone());
        map.insert_with_topic(SESSION3, topic_b, tx);

        // Drop one from topic A
        assert!(map.drop(SESSION1));

        // Should be removed from all mappings
        assert!(map.topic(SESSION1).is_none());
        assert!(!map.session_tx_map.contains_key(&SESSION1));

        // Remaining sessions for topic A
        let sessions = map.sessions(&topic_a);
        assert_eq!(sessions, HashSet::from([SESSION2]));

        // Dropping a non-existent session returns false
        assert!(!map.drop(10));
    }

    #[test]
    fn drop_removes_empty_topic_bucket() {
        let (tx, _rx) = mpsc::channel::<()>(128);
        let mut map = SessionTopicMap::default();
        let topic_a = Topic::random();

        map.insert_with_topic(SESSION1, topic_a, tx);
        assert!(map.topic_session_map.contains_key(&topic_a));

        // Dropping the last session for the topic removes the bucket entirely,
        // not just its (now empty) member set.
        assert!(map.drop(SESSION1));
        assert!(!map.topic_session_map.contains_key(&topic_a));
    }

    #[test]
    fn retain_drops_closed_senders_and_bounds_the_map() {
        let mut map = SessionTopicMap::default();
        let topic_a = Topic::random();

        // Two live sessions and one whose receiver we drop (session ended).
        let (tx1, _rx1) = mpsc::channel::<()>(128);
        let (tx2, _rx2) = mpsc::channel::<()>(128);
        let (tx3, rx3) = mpsc::channel::<()>(128);
        map.insert_with_topic(SESSION1, topic_a, tx1);
        map.insert_with_topic(SESSION2, topic_a, tx2);
        map.insert_with_topic(SESSION3, topic_a, tx3);
        drop(rx3);

        let dropped = map.retain(|tx| !tx.is_closed());

        assert_eq!(dropped, 1);
        assert!(map.topic(SESSION1).is_some());
        assert!(map.topic(SESSION2).is_some());
        assert!(map.topic(SESSION3).is_none());
        assert!(!map.session_tx_map.contains_key(&SESSION3));
        assert_eq!(map.sessions(&topic_a), HashSet::from([SESSION1, SESSION2]));

        // Idempotent: nothing left to prune.
        assert_eq!(map.retain(|tx| !tx.is_closed()), 0);
    }

    #[test]
    fn insert_multiple_sessions_same_topic() {
        let (tx1, _rx1) = mpsc::channel::<()>(128);
        let (tx2, _rx2) = mpsc::channel::<()>(128);
        let mut map = SessionTopicMap::default();

        let topic_a = Topic::random();

        map.insert_with_topic(SESSION1, topic_a, tx1);
        map.insert_with_topic(SESSION2, topic_a, tx2);

        let sessions = map.sessions(&topic_a);
        assert_eq!(sessions, HashSet::from([SESSION1, SESSION2]));
    }
}
