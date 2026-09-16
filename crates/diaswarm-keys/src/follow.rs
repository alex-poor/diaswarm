//! Joining somebody else's group, and reading what they granted you.
//!
//! **THIS LIVED IN `diaswarm-android` AND IS NOT ANDROID.** Every line of it is
//! `diaswarm-keys` over `p2panda-store`: there is no JNI, no NDK and nothing
//! platform-specific. It sat in the JNI crate only because the follower was the
//! first thing that needed it, and the desktop peer — D29's carrier, which is
//! about to become a reader too — could not reach it without copying it.
//!
//! **A SECOND COPY OF THIS WOULD BE THE WRONG KIND OF BUG.** It is the code
//! that decides which segments a reader is shown and which it is silently not,
//! and this project has twice paid for two implementations of one rule drifting
//! apart. One reader, two callers.
//!
//! Async here, `block_on` at the edges: the JNI has a runtime handle and the
//! peer has a runtime, and neither should be imposed on the other.

use std::collections::HashSet;
use std::path::Path;

use diaswarm_core::Record;
use p2panda_core::{SigningKey, VerifyingKey};

use crate::{Segment, SqliteStore, Vault, wire};

/// The distinct epochs a run of segments covers, oldest first.
///
/// **ONE EPOCH IS MANY SEGMENTS AND THEY ALL COUNT.** Each publish carries only
/// the records added since the last one, so a day is the concatenation of its
/// segments rather than the last of them. An earlier version kept only the
/// newest per epoch, which was right when each publish carried the whole day
/// and silently discards 99% of it now.
pub fn epochs_of(segments: &[Segment]) -> Vec<i64> {
    let mut seen: Vec<i64> = segments.iter().map(|s| s.epoch).collect();
    seen.sort_unstable();
    seen.dedup();
    seen
}

/// Join a subject's group using a welcome that has already replicated to us.
///
/// **ONE FIELD, BOTH KEYS.** Taking the author and the bundle separately let a
/// caller pair one subject's log with another's bundle — a mismatch that
/// produces no error, just a reader that never finds a welcome. They arrive
/// together in the invite and they stay together here.
pub async fn join(
    own_dir: &Path,
    joined_dir: &Path,
    store: &SqliteStore,
    signing: &SigningKey,
    keys_hex: &str,
    purpose: &str,
    offset_ms: i64,
) -> Result<(Vault, VerifyingKey), String> {
    let identity = crate::decode_identity(keys_hex).map_err(|e| format!("identity {e}"))?;
    let subject = identity.signer;
    let subject_bundle = identity.bundle.clone();

    // The identity this device published, not a fresh one.
    let own = Vault::open(own_dir, offset_ms, signing).map_err(|e| format!("own vault {e}"))?;
    let manager = own.manager_state().map_err(|e| format!("own identity {e}"))?;
    drop(own);

    let control = wire::control_from(store, &subject, None)
        .await
        .map_err(|e| format!("control log {e}"))?;
    if control.is_empty() {
        return Err("no grant has arrived yet".to_string());
    }

    let registry = Vault::registry(&[(crate::group::GrantTag::own(&subject), subject_bundle.clone())])
        .map_err(|e| format!("registry {e}"))?;

    for message in &control {
        let Ok(mut candidate) = Vault::open(joined_dir, offset_ms, signing) else { continue };
        if candidate
            .join(manager.clone(), registry.clone(), &subject_bundle, purpose, message)
            .is_ok()
        {
            return Ok((candidate, subject));
        }
    }
    Err(format!("none of the {} control messages welcome us", control.len()))
}

/// What a joined vault can open, deduplicated, with the counts that say how
/// much it could not.
///
/// **SEGMENTS COME FROM THE LOG, NOT A DIRECTORY.** A follower's arrive as
/// operation bodies over `p2panda-net` and never touch the filesystem, which is
/// the whole shape of D26 — so this reads them out of the store rather than
/// calling `read_from`. What it cannot open it counts; a short answer must
/// never be a silent one.
///
/// **`tail_days` IS THE FLAGSHIP'S PARAMETER, AND 0 IS NOT.** A parent needs 24
/// hours (D11), and `segments_tail` answers that by sequence number so the
/// store does the skipping — measured flat in `diaswarm-keys/tests/wire.rs` as
/// the log grows. `segments_from` reads the whole log and filters, which is
/// linear in everything the subject ever sealed: the right primitive for a
/// research export and the wrong one for a follower refreshing every two
/// minutes. Passing 0 asks for that linear read deliberately.
///
/// Returns `(records, opened, skipped)`.
///
/// 🔴 **`Skipped` IS SPLIT, AND THE SPLIT IS THE WHOLE POINT FOR A CLINICIAN
/// SCREEN.** `not_ours` is access control working — a segment sealed under a
/// secret this reader was never given, which is what "outside your window"
/// means. `lost()` is a failure: the secret was held and the bytes still would
/// not open, or the segment would not parse.
///
/// This used to return one number for both, and on 2026-09-16 that number
/// climbed all afternoon because a rotation had not been announced. A screen
/// reading it would have told a clinician "outside your window" about data that
/// was squarely inside it. **A reader may only say the access control refused
/// it when the access control actually did.**
pub async fn read(
    vault: &Vault,
    store: &SqliteStore,
    subject: &VerifyingKey,
    tail_days: u64,
    from_epoch: i64,
) -> Result<(Vec<Record>, usize, crate::Skipped), String> {
    // **ONE EPOCH IS MANY OPERATIONS, AND ONLY THE LAST ONE MATTERS.**
    //
    // A subject publishes on every flush — every five minutes — and each flush
    // re-seals the whole accumulated day, so the operations for one epoch are a
    // sequence of supersets and the newest contains all of them. That breaks
    // the invariant `segments_tail` was written under, which was "the last N
    // entries are the last N days": at a five-minute cadence the last two
    // entries are ten minutes of today.
    //
    // So the tail is asked for in *operations* rather than days — 288 flushes
    // to a day, plus slack for a re-drain — and then reduced by epoch.
    //
    // The cost is fetching supersets that are then discarded. It is the price
    // of a follower seeing today as it happens rather than after midnight, and
    // it is the thing to revisit first if replication gets expensive.
    let segments: Vec<Segment> = if tail_days > 0 {
        let entries = tail_days.saturating_mul(320).min(20_000);
        let tail = wire::segments_tail(store, subject, entries)
            .await
            .map_err(|e| format!("segments {e}"))?;
        // **AND THEN THE NEWEST `tail_days` OF THEM.** Asking the log for 320
        // operations a day is how many entries to *fetch*; it says nothing
        // about how many days those entries cover, and on a quiet log they
        // cover far more. A follower asking for one day must get one day —
        // `a_tail_read_returns_the_newest_days_and_no_others` caught this
        // returning thirty.
        let run: Vec<Segment> = tail.into_iter().filter(|s| s.epoch >= from_epoch).collect();
        let epochs = epochs_of(&run);
        let keep: HashSet<i64> =
            epochs.iter().rev().take(tail_days as usize).copied().collect();
        run.into_iter().filter(|s| keep.contains(&s.epoch)).collect()
    } else {
        wire::segments_from(store, subject, from_epoch)
            .await
            .map_err(|e| format!("segments {e}"))?
    };

    let mut out: Vec<Record> = Vec::new();
    let mut opened = 0usize;
    let mut skipped = crate::Skipped::default();
    // **DEDUPED, BECAUSE A RE-DRAIN REPUBLISHES.** Deltas do not normally
    // overlap, but a subject that re-reads its whole database seals the same
    // records again, and a reader must not show a reading twice.
    let mut seen: HashSet<String> = HashSet::new();
    for segment in &segments {
        match vault.open_segment(segment) {
            Ok((records, unparseable)) => {
                opened += 1;
                skipped.unparseable += unparseable;
                for record in records {
                    if seen.insert(record.to_canonical_json()) {
                        out.push(record);
                    }
                }
            }
            // NOT AN ERROR. A segment sealed under a secret this reader was
            // never given — before its grant, after a revocation, or outside a
            // scoped window — is the access control working.
            Err(crate::Error::NotGranted) => skipped.not_ours += 1,
            // A FAILURE, AND NOT THE SAME THING. The secret was held and the
            // ciphertext still would not open.
            Err(crate::Error::Undecryptable) => skipped.undecryptable += 1,
            Err(_) => skipped.unreadable += 1,
        }
    }
    Ok((out, opened, skipped))
}
