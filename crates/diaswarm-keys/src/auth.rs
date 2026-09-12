//! Who may grant, and the record that says they did.
//!
//! **[D13](../../docs/decisions.md) IS ALREADY BUILT HERE — IT JUST WAS NOT
//! BEING CHECKED.** That decision asks for a grant log that is signed per
//! entry, hash-chained so an altered or removed entry is detectable from one
//! copy, replicated so that truncating the local copy becomes *equivocation*
//! rather than deletion, and that names nobody. A p2panda log is all four of
//! those by construction:
//!
//! | D13 asks for | where it already is |
//! |---|---|
//! | a signature per entry | the operation header, checked by [`crate::wire::open_control`] |
//! | a hash chain | `backlink` + `seq_num` — **this module** |
//! | names nobody | [`crate::group::GrantTag`] |
//! | replicates to peers | `diaswarm-net`'s log sync, over `CONTROL_LOG_ID` |
//!
//! Only the third row needed writing, and until it was written the second
//! column was a claim about a data structure rather than something anything
//! verified. `p2panda_core::validate_operation` checks a signature and a
//! payload hash; it says nothing about whether this operation follows the one
//! before it.
//!
//! **WHAT THIS DELIBERATELY DOES NOT DO IS ENFORCE ANYTHING.** In
//! `diaswarm-core` the log *is* the permission: `entitled(tag)` answers which
//! segments a reader may read by asking which grant was live when. Here that
//! question has no meaning — a reader can open a segment if it holds the secret
//! that segment names, and cannot if it does not. Entitlement is cryptographic
//! and the log is for **accountability**: what the subject did, in an order,
//! that they cannot quietly revise. Those are different jobs and it is worth
//! not confusing them, because a log that enforced nothing but was treated as
//! if it did would be the worst of both.
//!
//! **AND ONE SUBJECT IS ONE SIGNING KEY.** "Who may grant" has a short answer
//! because a group's control log is a single author's log: the subject's. A
//! second device is a second key and therefore a second subject, not a second
//! voice in this one. That is a real limit — it is why there is no
//! `p2panda-auth` here — and it is the right trade while the flagship is one
//! person's phone publishing their own data.

use p2panda_core::{Hash, VerifyingKey};
use p2panda_store::logs::LogStore;
use p2panda_store::SqliteStore;

use crate::wire::{KeysOperation, CONTROL_LOG_ID};
use crate::Error;

type LogId = u32;
type SeqNum = u32;

/// One walk of one copy of a subject's control log.
///
/// Two of these, from two peers, are what makes equivocation visible — see
/// [`Chain::compare`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chain {
    /// The hash of every entry, in sequence order.
    ///
    /// Kept in full rather than reduced to a head, because a head alone cannot
    /// tell "shorter" from "different": two peers holding three and five
    /// entries have different heads whether or not they agree. Grants are rare
    /// — one per reader, plus revocations — so this is a handful of hashes and
    /// not a thing to be clever about.
    pub entries: Vec<Hash>,
    /// The first sequence number where the chain does not hold, if any.
    ///
    /// `Some(n)` means entry `n` does not follow entry `n - 1`: a wrong
    /// backlink, a skipped sequence number, or a different author. Mirrors
    /// `diaswarm_core::vault::Vault::verify_chain`, which returns the position
    /// of the first break for the same reason — "it is broken" is not
    /// actionable and "it is broken here" is.
    pub broken_at: Option<SeqNum>,
}

impl Chain {
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The last entry, which is what a peer would announce.
    pub fn head(&self) -> Option<Hash> {
        self.entries.last().copied()
    }

    pub fn is_intact(&self) -> bool {
        self.broken_at.is_none()
    }

    /// Do two copies of one subject's log tell the same story?
    ///
    /// **THIS IS THE HALF OF D13'S TAMPER-EVIDENCE THAT ONE COPY CANNOT GIVE.**
    /// A chain check catches an entry altered or dropped from the middle.
    /// It cannot catch a truncated tail: what remains is a valid prefix, and
    /// the subject holds every key needed to re-sign a shorter log.
    ///
    /// What closes that is not a new mechanism, it is the swarm. Once peers
    /// hold the log, a subject showing a different history to different peers
    /// is *equivocating*, and the disagreement is the evidence. This is the
    /// comparison that turns two copies into that evidence.
    ///
    /// Two limits survive and should not be talked past: an entry created and
    /// dropped **before any peer saw it** leaves no trace anywhere — the
    /// guarantee is "what was seen is permanent", never "the log is complete" —
    /// and catching equivocation needs peers to actually compare, which is what
    /// Certificate Transparency calls gossip.
    pub fn compare(&self, other: &Chain) -> Agreement {
        for (i, (a, b)) in self.entries.iter().zip(other.entries.iter()).enumerate() {
            if a != b {
                return Agreement::Forked { at: i as SeqNum };
            }
        }
        Agreement::Consistent { shared: self.len().min(other.len()) as SeqNum }
    }
}

/// What two copies of a log say about each other.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Agreement {
    /// Neither contradicts the other; one may simply hold fewer entries.
    ///
    /// **NOT THE SAME AS "BOTH ARE COMPLETE".** A peer that has seen less is
    /// consistent with one that has seen more, and so is a subject that
    /// truncated its tail before either peer saw the rest.
    Consistent { shared: SeqNum },
    /// Both claim an entry at this position and they are different entries.
    /// There is no innocent reading of this: the subject signed both.
    Forked { at: SeqNum },
}

/// Walk a subject's control log and check that each entry follows the last.
///
/// Catches an entry **altered or removed from the middle**, which a per-entry
/// signature alone does not — that is the whole point of the chain, and
/// `validate_operation` does not look at it.
///
/// Reads the whole log deliberately. The chain is only a chain if it is walked
/// from the start, and grants are rare enough that "the whole log" is a handful
/// of entries — unlike the segment log, where reading everything is the cost
/// D26 exists to avoid.
pub async fn verify_control_chain(
    store: &SqliteStore,
    subject: &VerifyingKey,
) -> Result<Chain, Error> {
    let entries = <SqliteStore as LogStore<
        KeysOperation,
        VerifyingKey,
        LogId,
        SeqNum,
        Hash,
    >>::get_log_entries(store, subject, &CONTROL_LOG_ID, None, None)
    .await?;

    let operations: Vec<KeysOperation> =
        entries.into_iter().flatten().map(|(op, _encoded_header)| op).collect();

    let mut hashes = Vec::with_capacity(operations.len());
    let mut broken_at = None;

    for (i, operation) in operations.iter().enumerate() {
        let seq = i as SeqNum;

        // The signature and the payload binding, first: a chain of entries
        // nobody signed links nothing to nothing.
        let valid = p2panda_core::validate_operation(operation).is_ok()
            && &operation.header.verifying_key == subject
            && operation.header.seq_num == seq;

        let linked = if i == 0 {
            // A first entry claims no backlink. One that does is claiming a
            // predecessor this log does not have.
            operation.header.backlink.is_none()
        } else {
            p2panda_core::validate_backlink(&operations[i - 1].header, &operation.header).is_ok()
        };

        if !(valid && linked) {
            broken_at = Some(seq);
            break;
        }
        hashes.push(operation.hash);
    }

    Ok(Chain { entries: hashes, broken_at })
}
