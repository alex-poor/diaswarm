//! Segments as p2panda operations, so replication is `p2panda-net`'s.
//!
//! **THE WORRY THIS ANSWERS.** [D26](../../docs/decisions.md) keeps
//! diaswarm's segments, and segments are not operations — so the obvious
//! reading is that it resurrects `wire.rs`, the bespoke pull protocol
//! [D21](../../docs/decisions.md) set out to delete. It does not, because the
//! cost that made `p2panda-spaces` unusable was never in the operation.
//!
//! `Ingested` has always separated `processing` — inside the spaces CRDT — from
//! `persisting`, writing that CRDT's state back. Both grew quadratically. A
//! carrier's entire job in `replicate.rs` is one `insert_operation`, a SQL
//! insert, and it was never implicated.
//!
//! So an operation carrying a sealed segment in its **body** is an envelope.
//! The header names the epoch, the secret and the nonce; the body is the
//! ciphertext; and opening it needs the body and the secret it names and
//! nothing else. No dependencies, no CRDT, no state that grows.
//!
//! `spike/p2panda-logsync` already measured that bodies replicate intact to at
//! least 4 MB. What was missing was whether the envelope itself costs anything
//! once a vault holds a lot of them, and that is what this module lets us ask.

use p2panda_core::{Body, Hash, Header, SigningKey, VerifyingKey};
use p2panda_encryption::crypto::xchacha20::XAeadNonce;
use p2panda_encryption::data_scheme::GroupSecretId;
use p2panda_store::logs::LogStore;
use p2panda_store::operations::OperationStore;
use p2panda_store::{SqliteError, SqliteStore, tx};
use serde::{Deserialize, Serialize};

use crate::Segment;

/// One log per subject: a device publishes its own records and nobody else's.
pub const LOG_ID: u32 = 0;

type LogId = u32;
type SeqNum = u32;

/// What a segment needs in the header so a reader can open its body.
///
/// **SMALL ON PURPOSE.** `p2panda-core` decodes headers with a 512-byte limit,
/// and D20 measured what happens when an application payload is put there
/// instead of in the body: 2.71 MB became 10,569 operations and a 27-minute
/// read. This is three fixed-size fields.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SegmentArgs {
    pub epoch: i64,
    pub secret_id: GroupSecretId,
    pub nonce: XAeadNonce,
}

pub type SegmentOperation = p2panda_core::Operation<SegmentArgs>;

/// Append a segment to a log as an operation, body and all.
pub async fn publish(
    store: &SqliteStore,
    signing_key: &SigningKey,
    segment: &Segment,
) -> Result<SegmentOperation, SqliteError> {
    let args = SegmentArgs {
        epoch: segment.epoch,
        secret_id: segment.secret_id,
        nonce: segment.nonce,
    };
    let payload = segment.ciphertext.clone();

    // ONE TRANSACTION for read-then-append: two concurrent publishes that read
    // the same latest entry would claim the same seq_num and backlink, which is
    // a forked log, which is not a recoverable state.
    let operation = tx!(store, {
        let (seq_num, backlink) = <SqliteStore as LogStore<
            SegmentOperation,
            VerifyingKey,
            LogId,
            SeqNum,
            Hash,
        >>::get_latest_entry_tx(store, &signing_key.verifying_key(), &LOG_ID)
        .await?
        .map(|op| (op.header.seq_num + 1, Some(op.hash)))
        .unwrap_or((0, None));

        let header = Header::builder()
            .seq_num(seq_num)
            .backlink(backlink)
            .body(&payload)
            .build(signing_key, args);
        let operation = SegmentOperation::from_parts(header, Some(Body::from_bytes(payload)));
        store.insert_operation(&operation.hash, &operation, &LOG_ID).await?;
        operation
    });
    Ok(operation)
}

/// The last `count` segments in a log.
///
/// **BY SEQUENCE NUMBER, BECAUSE FILTERING IN RUST IS NOT SKIPPING.** The first
/// version of this read the whole log and dropped what it did not want, and
/// measured linear in the log's length — 0.67 ms at seven days, 17.1 ms at a
/// hundred and eighty — which would have handed D26 back the problem it exists
/// to avoid.
///
/// The on-disk layout gets away with a filter because the epoch is in the
/// filename and the directory is the index. A log has no such index, but it has
/// `after`: `get_log_entries` takes a sequence-number range and the store does
/// the skipping. Segments are appended in epoch order, so "the last N days" is
/// "the last N entries", which is what a follower actually wants.
pub async fn segments_tail(
    store: &SqliteStore,
    author: &VerifyingKey,
    count: u64,
) -> Result<Vec<Segment>, SqliteError> {
    let heights = <SqliteStore as LogStore<
        SegmentOperation,
        VerifyingKey,
        LogId,
        SeqNum,
        Hash,
    >>::get_log_heights(store, author, &[LOG_ID])
    .await?;
    let tip = heights.and_then(|h| h.get(&LOG_ID).copied()).unwrap_or(0);
    let after = (tip as u64).saturating_sub(count.saturating_sub(1));
    let after = if after == 0 { None } else { Some((after - 1) as SeqNum) };

    let entries = <SqliteStore as LogStore<
        SegmentOperation,
        VerifyingKey,
        LogId,
        SeqNum,
        Hash,
    >>::get_log_entries(store, author, &LOG_ID, after, None)
    .await?;
    Ok(collect(entries))
}

/// Every segment in a log from `from_epoch` onwards.
///
/// Reads the whole log and filters, so it is linear in the log's length. Use
/// [`segments_tail`] for a follower; this is for a reader that genuinely wants
/// everything, where linear is the floor anyway.
pub async fn segments_from(
    store: &SqliteStore,
    author: &VerifyingKey,
    from_epoch: i64,
) -> Result<Vec<Segment>, SqliteError> {
    // NO `tx!` HERE, AND THAT IS NOT AN OVERSIGHT. The permit `tx!` takes is
    // for read-then-append; wrapping a plain read in one exhausts the pool and
    // the symptom is `PoolTimedOut`, which reads like a hung database rather
    // than a macro used where it does not belong.
    let entries = <SqliteStore as LogStore<
        SegmentOperation,
        VerifyingKey,
        LogId,
        SeqNum,
        Hash,
    >>::get_log_entries(store, author, &LOG_ID, None, None)
    .await?;
    Ok(collect(entries).into_iter().filter(|s| s.epoch >= from_epoch).collect())
}

fn collect(entries: Option<Vec<(SegmentOperation, Vec<u8>)>>) -> Vec<Segment> {
    entries
        .map(|e| e.into_iter())
        .into_iter()
        .flatten()
        .map(|(op, body)| Segment {
            epoch: op.header.extensions.epoch,
            secret_id: op.header.extensions.secret_id,
            nonce: op.header.extensions.nonce,
            ciphertext: body,
        })
        .collect()
}
