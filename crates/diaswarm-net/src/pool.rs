//! The pool: who holds which chunk, decided by arithmetic rather than by anyone.
//!
//! WHAT THIS IS FOR. Turning swarm on should mean your ciphertext goes into a
//! pool, and that everyone else who turned it on ends up holding pieces of it.
//! Not because they were asked, and not because they know you — because the
//! assignment says so. That is what makes the pool available when the people
//! you actually know are all asleep, and what makes it heal when one of them
//! throws their phone in a river.
//!
//! **RENDEZVOUS HASHING, and the reason is self-healing.** For each chunk,
//! every peer computes `H(peer ‖ chunk)` and the highest R scores are the
//! holders. Nobody coordinates, nobody is elected, and there is no register of
//! who has what: every peer computes the same answer from the same inputs.
//!
//! The property that matters is what happens when a peer vanishes. Its chunks
//! were the ones where it ranked first; with it gone the peer that ranked
//! second now ranks first, and adopts them on its next pass. Nothing else
//! moves. A repair process would have to notice the loss, decide who fixes it,
//! and avoid two peers fixing it twice — this has none of those, because the
//! ranking already contains the answer.
//!
//! Consistent hashing with a ring would do the same job. Rendezvous is chosen
//! because it needs no virtual nodes to spread load evenly, and the whole
//! calculation is a hash per peer per chunk.
//!
//! **A CHUNK IS A SEGMENT.** Already the unit of key custody, already
//! addressed by `(seq, epoch)`, already independently openable given its wrap.
//! Splitting finer would mean a reader could hold half of something it cannot
//! read.
//!
//! WHAT THIS MODULE DOES NOT DO: fetch anything, or decide what a peer can
//! read. It answers one question — "should I be holding this?" — and holding
//! is not reading. A peer holds ciphertext for people it has never met and
//! cannot open a byte of it.

use std::path::{Path, PathBuf};

use anyhow::Result;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// A peer this one has met.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Peer {
    /// Endpoint id, optionally `id@addr,addr` for a local network.
    pub at: String,
    /// When it was last known to be there, in epoch milliseconds.
    pub seen: i64,
}

impl Peer {
    /// The endpoint id alone — what every ranking is computed over.
    ///
    /// Addresses must not enter the hash: the same phone on wifi and on mobile
    /// data would otherwise be two different peers, and everything it carried
    /// would reshuffle when it walked out of the house.
    pub fn id(&self) -> &str {
        self.at.split('@').next().unwrap_or(&self.at)
    }
}

fn peers_path(store: &Path) -> PathBuf {
    store.join("peers.json")
}

pub fn peers(store: &Path) -> Vec<Peer> {
    std::fs::read(peers_path(store))
        .ok()
        .and_then(|b| serde_json::from_slice::<Vec<Peer>>(&b).ok())
        .unwrap_or_default()
}

pub fn save_peers(store: &Path, list: &[Peer]) -> Result<()> {
    std::fs::create_dir_all(store)?;
    std::fs::write(peers_path(store), serde_json::to_vec_pretty(list)?)?;
    Ok(())
}

/// How long a peer counts towards the pool after it was last seen.
///
/// Long enough that a phone in a pocket overnight is not evicted and its share
/// redistributed for no reason; short enough that a device which is actually
/// gone stops being counted on. Redistribution is not free — it is somebody
/// else's mobile data.
pub const FORGET_AFTER_MS: i64 = 7 * 24 * 60 * 60 * 1000;

/// Note that a peer exists, or that it still does.
pub fn remember_peer(store: &Path, at: &str, now: i64) -> Result<bool> {
    let id = at.split('@').next().unwrap_or("");
    if id.len() != 64 || !id.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Ok(false);
    }
    let mut list = peers(store);
    match list.iter_mut().find(|p| p.id() == id) {
        Some(existing) => {
            existing.seen = now;
            if at.len() > existing.at.len() {
                existing.at = at.to_string();
            }
            save_peers(store, &list)?;
            Ok(false)
        }
        None => {
            list.push(Peer { at: at.to_string(), seen: now });
            save_peers(store, &list)?;
            Ok(true)
        }
    }
}

/// Peers still counted as part of the pool.
pub fn live_peers(store: &Path, now: i64) -> Vec<Peer> {
    peers(store).into_iter().filter(|p| now - p.seen <= FORGET_AFTER_MS).collect()
}

/// How many peers should hold each bucket.
///
/// Three, matching feasibility.md's assumption that a member holds three
/// others' data. Two survives one loss and is one bad night from none.
pub const REPLICAS: usize = 3;

/// Which bucket a subject falls in, at a given depth.
///
/// **A PREFIX OF THE HASH, and that is what makes disagreement survivable.**
/// Peers choose their depth from an estimate of how big the pool is, and two
/// peers will sometimes disagree by a bit. Because a bucket is a prefix, a peer
/// at depth 6 holding bucket `101011` holds everything a peer at depth 7 would
/// file under `1010110` and `1010111`. A coarser peer is a superset of a finer
/// one, so a disagreement costs a little extra storage rather than a subject
/// nobody holds and nobody can find.
pub fn bucket_of(subject: &str, depth: u8) -> u64 {
    let h = Sha256::digest(subject.as_bytes());
    let top = u64::from_be_bytes(h[..8].try_into().expect("32 bytes"));
    if depth == 0 { 0 } else { top >> (64 - depth.min(63) as u32) }
}

/// How finely to cut the subject space for a pool of this size.
///
/// Roughly one bucket per peer. Coarser and each peer carries a slice of the
/// pool that grows with the pool — the fixed-size version is fine to a hundred
/// peers and then climbs without limit. Finer buys nothing.
pub fn depth_for(pool: usize) -> u8 {
    let mut d = 0u8;
    while (1usize << d) < pool.max(1) && d < 32 {
        d += 1;
    }
    d
}

/// Roughly how many buckets a peer ends up carrying, for sizing and for tests.
/// The real assignment is [`holders_of_bucket`]; this is the expectation.
pub fn buckets_each(pool: usize, depth: u8, replicas: usize) -> usize {
    let total = 1usize << depth.min(31);
    if pool == 0 {
        return total;
    }
    // ceil(total * replicas / pool), never zero, never more than all of them.
    ((total * replicas).div_ceil(pool)).clamp(1, total)
}

/// Who carries a bucket: rank every peer for it, take the best few.
///
/// **BUCKET-CENTRIC, and the first version was not.** Having each peer choose
/// its own favourite buckets seems equivalent and is not: nothing then
/// guarantees that some bucket was chosen by anybody. Tested against a pool of
/// forty and a subject fell through with no holder at all — data accepted and
/// then lost, which is the one outcome that cannot be allowed.
///
/// Ranking peers per bucket makes coverage automatic. Every bucket gets
/// exactly `min(replicas, peers)` holders because that is what taking the top
/// few of a total order means.
pub fn holders_of_bucket(peers: &[String], bucket: u64, replicas: usize) -> Vec<String> {
    let mut ranked: Vec<(&String, [u8; 32])> =
        peers.iter().map(|p| (p, bucket_score(p, bucket))).collect();
    ranked.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(b.0)));
    ranked.into_iter().take(replicas).map(|(p, _)| p.clone()).collect()
}

/// The buckets this peer carries.
///
/// Costs one hash per peer per bucket, and depth tracks pool size, so this is
/// quadratic in the size of the pool. Fine for the hundreds this is built for;
/// a pool in the tens of thousands would want arcs over a range of the hash
/// space rather than enumerated buckets.
pub fn my_buckets(me: &str, peers: &[String], depth: u8, replicas: usize) -> Vec<u64> {
    let total = 1u64 << depth.min(31);
    (0..total)
        .filter(|b| holders_of_bucket(peers, *b, replicas).iter().any(|p| p == me))
        .collect()
}

/// Does this peer carry the bucket a subject lives in?
pub fn holds_subject(me: &str, peers: &[String], subject: &str, replicas: usize) -> bool {
    let depth = depth_for(peers.len());
    let bucket = bucket_of(subject, depth);
    holders_of_bucket(peers, bucket, replicas).iter().any(|p| p == me)
}

fn bucket_score(peer_id: &str, bucket: u64) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(peer_id.as_bytes());
    h.update(b"\0bucket\0");
    h.update(bucket.to_be_bytes());
    h.finalize().into()
}

/// The gossip topic for a bucket, as 32 bytes.
///
/// Depth is mixed in so that a bucket at depth 6 and its child at depth 7 are
/// different topics, and a peer that has split its buckets is not still
/// listening to the coarse one by accident.
pub fn bucket_topic(depth: u8, bucket: u64) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(b"diaswarm-bucket-v1");
    h.update([depth]);
    h.update(bucket.to_be_bytes());
    h.finalize().into()
}

/// The topic every peer joins, carrying no data.
///
/// Its only job is to be somewhere to count: pool size decides how finely the
/// subject space is cut and how many buckets each peer carries, and without a
/// shared place to observe membership every peer would guess differently.
pub fn presence_topic() -> [u8; 32] {
    Sha256::digest(b"diaswarm-presence-v1").into()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pool_ids(n: usize) -> Vec<String> {
        (0..n)
            .map(|i| {
                let h = Sha256::digest(format!("peer-{i}").as_bytes());
                h.iter().map(|b| format!("{b:02x}")).collect()
            })
            .collect()
    }

    fn subjects(n: usize) -> Vec<String> {
        (0..n)
            .map(|i| {
                let h = Sha256::digest(format!("subject-{i}").as_bytes());
                h.iter().map(|b| format!("{b:02x}")).collect()
            })
            .collect()
    }

    /// EVERY SUBJECT IS HELD BY SOMEBODY. The one that cannot be allowed to
    /// fail: a subject nobody carries is data that has been accepted and lost.
    #[test]
    fn nothing_falls_through_the_gaps() {
        for n in [1usize, 2, 3, 5, 10, 40, 100] {
            let peers = pool_ids(n);
            let depth = depth_for(n);
            for s in subjects(200) {
                let holders = peers.iter().filter(|p| holds_subject(p, &peers, &s, REPLICAS)).count();
                assert_eq!(holders, REPLICAS.min(n), "pool of {n}: wrong number of holders");
            }
        }
    }

    /// And held enough times to survive a loss.
    #[test]
    fn each_subject_is_held_about_the_replication_factor_of_times() {
        for n in [5usize, 10, 40, 100] {
            let peers = pool_ids(n);
            let mut thin = 0;
            for s in subjects(200) {
                let holders = peers.iter().filter(|p| holds_subject(p, &peers, &s, REPLICAS)).count();
                if holders < REPLICAS { thin += 1; }
            }
            // Rendezvous over buckets is not exact — a handful land short.
            assert_eq!(thin, 0, "pool of {n}: {thin}/200 subjects under-replicated");
        }
    }

    /// A tiny pool holds everything rather than refusing.
    #[test]
    fn two_peers_each_hold_everything() {
        let peers = pool_ids(2);
        for s in subjects(50) {
            let holders = peers.iter().filter(|p| holds_subject(p, &peers, &s, REPLICAS)).count();
            assert_eq!(holders, 2, "with two peers both should hold everything");
        }
    }

    /// WHAT MAKES IT AFFORDABLE. Each peer's share must stay roughly constant
    /// as the pool grows — otherwise being an early member becomes a tax, and
    /// the whole thing stops being something a phone can do.
    #[test]
    fn a_peers_share_does_not_grow_with_the_pool() {
        let mut shares = vec![];
        for n in [10usize, 40, 100, 400] {
            let depth = depth_for(n);
            let total = 1usize << depth;
            let each = buckets_each(n, depth, REPLICAS);
            // Fraction of the whole subject space one peer carries.
            shares.push(each as f64 / total as f64 * n as f64);
        }
        for s in &shares {
            assert!(
                *s > 1.0 && *s < REPLICAS as f64 * 2.5,
                "a peer's share works out at {s} subjects-worth, expected around {REPLICAS}: {shares:?}"
            );
        }
    }

    /// SELF-HEALING. When the pool shrinks, survivors must cover more — that is
    /// the whole repair mechanism, and it is arithmetic rather than a process.
    #[test]
    fn a_smaller_pool_makes_every_survivor_carry_more() {
        let depth = 8;
        let many = pool_ids(20);
        let few: Vec<String> = many.iter().take(10).cloned().collect();
        let me = &few[0];
        let before = my_buckets(me, &many, depth, REPLICAS).len();
        let after = my_buckets(me, &few, depth, REPLICAS).len();
        assert!(after > before, "losing half the pool did not widen anyone: {before} -> {after}");
    }

    /// A coarser peer is a superset of a finer one, so disagreeing about pool
    /// size costs storage rather than losing a subject.
    #[test]
    fn a_bucket_is_a_prefix_so_depth_disagreement_is_survivable() {
        for s in subjects(100) {
            let coarse = bucket_of(&s, 6);
            let fine = bucket_of(&s, 7);
            assert_eq!(fine >> 1, coarse, "depth 7 is not a refinement of depth 6");
        }
    }

    #[test]
    fn a_peer_keeps_its_buckets_when_the_pool_merely_grows() {
        // Growing the pool should narrow what a peer carries, not shuffle it
        // onto a different part of the space — otherwise every join re-syncs
        // everybody.
        let small = pool_ids(10);
        let big = pool_ids(40);
        let me = &small[0];
        assert_eq!(me, &big[0], "the same peer must be present in both pools");
        let wide = my_buckets(me, &small, 8, REPLICAS);
        let narrow = my_buckets(me, &big, 8, REPLICAS);
        assert!(narrow.len() < wide.len(), "growing the pool did not narrow the share");
        for b in &narrow {
            assert!(wide.contains(b), "bucket {b} appeared that was not held before");
        }
    }

    #[test]
    fn topics_differ_by_depth_and_bucket() {
        assert_ne!(bucket_topic(6, 3), bucket_topic(7, 3));
        assert_ne!(bucket_topic(6, 3), bucket_topic(6, 4));
        assert_ne!(bucket_topic(6, 3), presence_topic());
    }

    #[test]
    fn addresses_do_not_change_who_holds_what() {
        let bare = Peer { at: "aa".repeat(32), seen: 0 };
        let with_addr = Peer { at: format!("{}@192.168.1.5:1234", "aa".repeat(32)), seen: 0 };
        assert_eq!(bare.id(), with_addr.id());
    }
}
