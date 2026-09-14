//! Carrying a share of the pool: one implementation, for every kind of peer.
//!
//! **THIS WAS INSIDE A JNI FUNCTION, AND A DESKTOP PEER WOULD HAVE COPIED IT.**
//! `keysCarryAll` in `diaswarm-android` worked out what a phone should hold —
//! its own subject, the subjects it follows, and strangers heard in its share
//! of the bucket space — carried each of them, and reported back what it held
//! so the next pool tick would announce it ([D28](../../docs/decisions.md)).
//! All of that is a property of *being a peer*, not of being Android, and the
//! second caller was about to be a desktop daemon ([D29](../../docs/decisions.md)).
//!
//! Two copies of this would be two copies of the rule about what a peer owes
//! the pool — and the standing preference in this project is explicit that
//! "two mechanisms that can disagree" is how its defects have arrived. So the
//! JNI keeps the handle juggling and the error codes, which are genuinely
//! Android's, and the decision lives here.

use std::path::Path;

use anyhow::Result;

use crate::replicate::KeysReplicator;
use crate::swarm::Swarm;

/// What one pass over the pool did.
///
/// **`carrying` IS A TOTAL AND `adopted` IS A DELTA, AND THE FIRST VERSION GOT
/// THAT WRONG.** It reported the length of the *wanted* list, which shrinks as
/// soon as a subject is taken on — because `keys_wanted` excludes what is
/// already held. On a real pool that printed `carrying=1`, then `4`, then `1`
/// again, which reads as a peer dropping three subjects it had just adopted.
/// Nothing had been dropped. A per-pass count wearing the name of a total is
/// the same mistake as the live counter, in a different file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Share {
    /// Every subject this peer carries, from the replicator's own association
    /// set — not from anything this function counted.
    pub carrying: usize,
    /// Subjects taken on during this pass, which may be zero on a healthy peer
    /// that already holds its share.
    pub adopted: usize,
}

/// Carry this peer's share: its own subject, what it follows, and strangers.
///
/// **`max_adopt` IS THE CALLER'S, AND `0` IS A REAL ANSWER** — it means "serve
/// what I already hold and take on nothing new". A peer passing zero is still a
/// full member: it announces and serves, so somebody it follows still gains a
/// holder. What it does not do is start storing strangers. Ayni passed zero for
/// a day and the result was an app named for reciprocity that carried nothing
/// for anybody; see D28 for why the announcing and adopting halves belong
/// together.
///
/// Bounded per pass because the first caller was a phone in a worker with a
/// deadline: a peer joining a large pool catches up over passes rather than
/// pulling its whole share at once.
pub async fn carry_share(
    swarm: &Swarm,
    replicator: &KeysReplicator,
    store: &Path,
    own_subject: &str,
    max_adopt: usize,
) -> Result<Share> {
    // `.max(2)` because a pool of one has no depth to speak of and the first
    // pass usually runs before anybody has been heard from.
    let members = swarm.pool_members().await.map(|m| m.len()).unwrap_or(2).max(2);
    let depth = crate::pool::depth_for(members);

    let mut wanted: Vec<String> = vec![own_subject.to_string()];

    // Everyone we follow, by the keys identity their invite carried. A follow
    // with no keys identity is skipped rather than failed: it is somebody
    // paired before the field existed, or a subject with no keys vault.
    for follow in crate::peer::load_follows(store).unwrap_or_default() {
        let Some(keys) = follow.keys.as_deref() else { continue };
        let Ok(id) = diaswarm_keys::decode_identity(keys) else { continue };
        wanted.push(id.signer.to_hex());
    }
    let mine = wanted.len();

    if max_adopt > 0 {
        let heard = swarm.keys_wanted().await.unwrap_or_default();
        wanted.extend(heard.into_iter().map(|(s, _)| s).take(max_adopt));
    }

    let before = replicator.carried().len();
    for subject in &wanted {
        let topic = crate::pool::bucket_topic(depth, crate::pool::bucket_of(subject, depth));
        let _ = replicator.carry(topic, subject).await;
    }
    let after = replicator.carried();

    // SAY WHAT WE NOW HOLD, OR NOBODY ELSE CAN FIND IT. A peer that fetches and
    // stays quiet is a dead end: the subject's own phone stays the only address
    // anyone can learn, which is the state D28 exists to end.
    swarm.set_keys_held(after.clone());

    let _ = mine;
    Ok(Share { carrying: after.len(), adopted: after.len().saturating_sub(before) })
}
