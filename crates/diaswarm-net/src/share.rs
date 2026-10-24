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
use crate::swarm::{is_subject, Swarm};

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
/// **RECIPROCITY IS THE MANDATORY PART, AND IT IS NOT THE BUDGET.**
/// [D30](../../docs/decisions.md): if you read it, you carry it. Follows are
/// added here *unconditionally* — `max_adopt` gates only the generous half,
/// carrying for strangers. A reader that serves what it consumes is the rule
/// the project is named after, and on this vault it is not even enforceable
/// policy: a reader reads from its own store, and `carry` is what puts anything
/// there. You cannot read a log you do not hold.
///
/// Generosity is also what keeps the *interest* leak bounded, which is easy to
/// miss. A carrier announces what it holds, so a reader that must carry is a
/// reader that must announce — and "P holds Y" only stays ambiguous between
/// following Y and merely carrying it while somebody is doing the latter. A
/// pool where everyone passed `0` would publish its own social graph.
///
/// **None of that is about reading.** The data is public ciphertext by design
/// (D1) and holding is not reading: anyone may carry a subject and open none of
/// it. What is at stake here is who is *interested* in whom, which D19 already
/// accepted as visible.
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
    also: &[String],
) -> Result<Share> {
    // `depth`/`members` are gone with the legacy bucket topic below — the new
    // `subject_topic` rendezvous takes no depth (D31), which was the point.

    let mut wanted: Vec<String> = vec![own_subject.to_string()];

    // **SUBJECTS THIS PEER WAS TOLD TO CARRY, WHICH NEED NO GOSSIP AT ALL.**
    // Everything else here waits to be told: follows come from a pairing, and
    // strangers from a `HoldingKeys` announcement that has to arrive over an
    // overlay whose membership is sampled. A carrier somebody runs for their
    // own family should not be at the mercy of that — they already know whose
    // data they mean. Named subjects are carried every pass, unconditionally.
    wanted.extend(also.iter().filter(|s| is_subject(s)).cloned());

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
        // **THE RENDEZVOUS, AND IT HAS NO PARAMETERS.** A subject you were
        // granted is known by key; finding it is not a sharding problem. See
        // [D31](../../docs/decisions.md) and `pool::subject_topic`.
        let _ = replicator.carry(crate::pool::subject_topic(subject), subject).await;

        // **THE OLD BUCKET TOPIC IS GONE.** For the cutover to `subject_topic`
        // (D31) a subject was carried on BOTH the new per-subject topic and the
        // old depth-sharded `bucket_topic`, so nothing went dark while peers
        // upgraded ("DELETE once no peer on the old topic remains"). That moment
        // has arrived: the live pool is only our own devices and all are on the
        // new scheme. Removed because every carried topic is a gossip overlay
        // whose HyParView churn starts and tears down a sync session on each
        // NeighbourUp/Down, and p2panda-net leaks per teardown — so a redundant
        // third topic per subject was ~a third of the carrier's session-churn
        // leak for zero benefit. See the leak investigation in
        // `diaswarm-ayni-leak-is-reader-readcost` (memory). `depth`/`members`
        // went with it; re-add the dual-carry only for a genuine mixed-version
        // rollout, never as a permanent state.

        // **AND THE GRANTS ON A TOPIC OF THEIR OWN.** `subject_topic` above
        // carries both of a subject's logs, and `TopicLogSync` drains them in one
        // session, segments before grants. A subject with a long history offers
        // thousands of segment operations ahead of its handful of grants, so a
        // follower on a link that drops before the segments are through dies
        // inside them and never reaches the grant log — the welcome it was
        // granted stranded in a log it cannot get to (`control_from` empty,
        // "no grant has arrived yet", measured as 257/5063 segments and 0/76
        // control on a phone). Carrying the control log alone on its own topic
        // gives the grants a session that completes in one round trip whatever
        // the segment backlog is. Additive: the segments still arrive by the
        // topics above, exactly as before.
        let _ = replicator
            .carry_logs(
                crate::pool::control_topic(subject),
                subject,
                &[diaswarm_keys::wire::CONTROL_LOG_ID],
            )
            .await;
    }
    let after = replicator.carried();

    // SAY WHAT WE NOW HOLD, OR NOBODY ELSE CAN FIND IT. A peer that fetches and
    // stays quiet is a dead end: the subject's own phone stays the only address
    // anyone can learn, which is the state D28 exists to end.
    swarm.set_keys_held(after.clone());

    let _ = mine;
    Ok(Share { carrying: after.len(), adopted: after.len().saturating_sub(before) })
}
