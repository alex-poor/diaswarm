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
use p2panda_store::{SqliteStore, tx};
use serde::{Deserialize, Serialize};

use crate::group::{GrantTag, Message};
use crate::{Authentic, Error, Segment};

/// One log per subject: a device publishes its own records and nobody else's.
pub const LOG_ID: u32 = 0;

/// Control messages go in a **separate log**, and that is not tidiness.
///
/// [`segments_tail`] asks for the last N entries of [`LOG_ID`] and calls them
/// the last N days. That holds only because every entry in that log is one
/// day's segment. Interleaving grants would quietly make "the last seven
/// entries" mean fewer than seven days, and the failure would look like a
/// follower missing data rather than like a log-id decision.
///
/// Separate logs also mean a carrier can replicate segments without
/// replicating grants, and the other way round.
pub const CONTROL_LOG_ID: u32 = 1;

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

/// The only version there has been.
pub const CONTROL_V1: u8 = 1;

/// What rides in a control operation's header.
///
/// A version byte and nothing else. The message itself goes in the **body**,
/// for the same reason a segment's ciphertext does: `p2panda-core` decodes
/// headers with a 512-byte limit, and a welcome carrying a whole secret bundle
/// is not small.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ControlArgs {
    pub v: u8,
}

/// **ONE EXTENSION TYPE FOR BOTH LOGS, BECAUSE ONE `LogSync` CARRIES ONE.**
///
/// `p2panda_net::LogSync<S, L, E>` is generic over a single extension type, and
/// two instances cannot share an endpoint. A header is stored encoded, so a log
/// written as `SegmentArgs` cannot be read back as anything else — which means
/// the choice is made at publish time and is not revisitable.
///
/// So both logs carry this enum and the variant says which log the operation
/// belongs in. That is a runtime check where a type could have been, and the
/// trade is deliberate: it buys one sync session covering a subject's segments
/// *and* its grants, which is what a follower needs and what two sessions on
/// one endpoint cannot give.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum KeysArgs {
    Segment(SegmentArgs),
    Control(ControlArgs),
}

impl KeysArgs {
    /// Which log an arriving operation belongs in.
    ///
    /// A carrier is handed operations by log sync and has to store them
    /// somewhere; with one log that was a constant, and with two it is this.
    pub fn log_id(&self) -> u32 {
        match self {
            KeysArgs::Segment(_) => LOG_ID,
            KeysArgs::Control(_) => CONTROL_LOG_ID,
        }
    }
}

/// Every log a subject publishes to, for a carrier to associate with a topic.
pub const LOG_IDS: [u32; 2] = [LOG_ID, CONTROL_LOG_ID];

pub type KeysOperation = p2panda_core::Operation<KeysArgs>;

/// Append a segment to a log as an operation, body and all.
pub async fn publish(
    store: &SqliteStore,
    signing_key: &SigningKey,
    segment: &Segment,
) -> Result<KeysOperation, Error> {
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
            KeysOperation,
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
            .build(signing_key, KeysArgs::Segment(args));
        let operation = KeysOperation::from_parts(header, Some(Body::from_bytes(payload)));
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
) -> Result<Vec<Segment>, Error> {
    let heights = <SqliteStore as LogStore<
        KeysOperation,
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
        KeysOperation,
        VerifyingKey,
        LogId,
        SeqNum,
        Hash,
    >>::get_log_entries(store, author, &LOG_ID, after, None)
    .await?;
    collect(entries)
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
) -> Result<Vec<Segment>, Error> {
    // NO `tx!` HERE, AND THAT IS NOT AN OVERSIGHT. The permit `tx!` takes is
    // for read-then-append; wrapping a plain read in one exhausts the pool and
    // the symptom is `PoolTimedOut`, which reads like a hung database rather
    // than a macro used where it does not belong.
    let entries = <SqliteStore as LogStore<
        KeysOperation,
        VerifyingKey,
        LogId,
        SeqNum,
        Hash,
    >>::get_log_entries(store, author, &LOG_ID, None, None)
    .await?;
    Ok(collect(entries)?.into_iter().filter(|s| s.epoch >= from_epoch).collect())
}

/// **THE SECOND HALF OF A `LogEntries` TUPLE IS THE HEADER, NOT THE BODY.**
///
/// `p2panda_store`'s type is `Vec<(T, Vec<u8>)>` and its SQL selects `hash,
/// header, body` — and then pushes `(operation, header)`. The body is not
/// dropped; it is on the operation, as `op.body`. But the tuple reads exactly
/// like `(op, body)` and this function took it as one, so every `Segment` this
/// returned carried an encoded header where its ciphertext should have been.
///
/// Nothing caught it because nothing decrypted what came back out of the log:
/// `tests/wire.rs` counted segments and timed the read. The control path found
/// it immediately, because verifying a signature against the payload is a
/// comparison and counting is not — which is the argument for doing the
/// verification at all, quite apart from forgery.
fn collect(entries: Option<Vec<(KeysOperation, Vec<u8>)>>) -> Result<Vec<Segment>, Error> {
    let mut out = Vec::new();
    for (op, _encoded_header) in entries.into_iter().flatten() {
        let body = op.body.ok_or_else(|| {
            Error::Forged(format!(
                "segment operation at seq {} has no body",
                op.header.seq_num
            ))
        })?;
        // A control message in the segment log is not a segment with odd
        // fields, it is a log that has been written to by something that should
        // not have. Saying so beats decrypting a welcome and reporting it as
        // `undecryptable`.
        let KeysArgs::Segment(args) = &op.header.extensions else {
            return Err(Error::Forged(format!(
                "a control operation is sitting in the segment log at seq {}",
                op.header.seq_num
            )));
        };
        out.push(Segment {
            epoch: args.epoch,
            secret_id: args.secret_id,
            nonce: args.nonce,
            ciphertext: body.to_bytes(),
        });
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Control messages
// ---------------------------------------------------------------------------

/// Append a control message to the subject's control log, signed.
///
/// **THE SIGNATURE IS THE AUTHENTICATION.** Nothing else in the message says
/// who wrote it that cannot be typed by hand; `sender` is a field in a struct.
/// What makes it checkable is that the bytes are the body of an operation whose
/// header is signed, and [`open_control`] refuses anything else.
pub async fn publish_control(
    store: &SqliteStore,
    signing_key: &SigningKey,
    message: &Message,
) -> Result<KeysOperation, Error> {
    // The sender the message claims must be the one this key can prove. A
    // subject publishing somebody else's message would produce an operation
    // that `open_control` rejects, and catching it here says so plainly rather
    // than at the far end of a network.
    let expected = GrantTag::own(&signing_key.verifying_key());
    if message.sender() != expected {
        return Err(Error::Forged(format!(
            "message says it is from {} but this key is {expected}",
            message.sender()
        )));
    }
    let payload = p2panda_core::cbor::encode_cbor(message)
        .map_err(|e| Error::Encode(e.to_string()))?;

    // ONE TRANSACTION, for the reason `publish` gives: read-then-append.
    let operation = tx!(store, {
        let (seq_num, backlink) = <SqliteStore as LogStore<
            KeysOperation,
            VerifyingKey,
            LogId,
            SeqNum,
            Hash,
        >>::get_latest_entry_tx(store, &signing_key.verifying_key(), &CONTROL_LOG_ID)
        .await?
        .map(|op| (op.header.seq_num + 1, Some(op.hash)))
        .unwrap_or((0, None));

        let header = Header::builder()
            .seq_num(seq_num)
            .backlink(backlink)
            .body(&payload)
            .build(signing_key, KeysArgs::Control(ControlArgs { v: CONTROL_V1 }));
        let operation = KeysOperation::from_parts(header, Some(Body::from_bytes(payload)));
        store.insert_operation(&operation.hash, &operation, &CONTROL_LOG_ID).await?;
        operation
    });
    Ok(operation)
}

/// Check a control operation and, if it holds up, vouch for its message.
///
/// **THE ONLY CONSTRUCTOR OF [`Authentic`]**, which is what makes
/// [`crate::Vault::receive`] safe by construction rather than by inspection.
///
/// Three things are checked and they are not interchangeable:
///
/// 1. `validate_operation` — the header's signature verifies, and the body is
///    the one the header committed to. The operation is reassembled from the
///    body bytes first, because the store hands back header and payload
///    separately and validating a header with no payload attached would check
///    the signature without checking what it was over.
/// 2. the author is the key the caller named. A valid signature by *somebody*
///    is not authentication; it has to be the subject being followed.
/// 3. the `sender` inside the message is `GrantTag::own(author)`. Without this
///    a subject could sign a message claiming to be from one of its own
///    readers, and the group state would apply it as that reader's.
pub fn open_control(
    operation: KeysOperation,
    expected: &VerifyingKey,
) -> Result<Authentic, Error> {
    let author = operation.header.verifying_key;
    if &author != expected {
        return Err(Error::Forged(format!(
            "signed by {} not {}",
            &author.to_hex()[..16],
            &expected.to_hex()[..16]
        )));
    }

    // **THE BODY MUST BE PRESENT FOR THE CHECK TO MEAN ANYTHING.**
    // `validate_operation` compares the header's payload hash and size against
    // the payload it is given, and a `None` payload is not a mismatch — it is
    // nothing to compare. An operation whose body was pruned would sail through
    // and then decode as whatever the caller happened to pass.
    if !matches!(operation.header.extensions, KeysArgs::Control(_)) {
        return Err(Error::Forged(
            "a segment operation was offered as a control message".to_string(),
        ));
    }
    let body = operation
        .body
        .as_ref()
        .ok_or_else(|| Error::Forged("control operation arrived with no body".to_string()))?
        .to_bytes();
    p2panda_core::validate_operation(&operation).map_err(|e| Error::Forged(e.to_string()))?;

    let message: Message = p2panda_core::cbor::decode_cbor(&body[..])
        .map_err(|e| Error::Encode(format!("control message will not decode: {e}")))?;

    let claimed = message.sender();
    let actual = GrantTag::own(&author);
    if claimed != actual {
        return Err(Error::Forged(format!(
            "signed by {actual} but the message says it is from {claimed}"
        )));
    }

    Ok(Authentic::vouched(author, message))
}

/// Every control message in `subject`'s log after `after`, each authenticated.
///
/// `after` is a sequence number, so catching up is "what has arrived since",
/// not "fetch everything and work out what is new". A vault that has processed
/// up to seq 3 asks for `Some(3)`.
///
/// **THE LINKS INSIDE WHAT IT RETURNS ARE CHECKED HERE**, so acting on a
/// control log that has had an entry swapped out is not something a caller can
/// do by forgetting to ask. What this cannot see is the run it was not given:
/// asking for `Some(3)` says nothing about entries 0 to 3. For the whole-log
/// guarantee — and for comparing two peers' copies — use
/// [`crate::auth::verify_control_chain`].
pub async fn control_from(
    store: &SqliteStore,
    subject: &VerifyingKey,
    after: Option<SeqNum>,
) -> Result<Vec<Authentic>, Error> {
    // NO `tx!`: a plain read, and wrapping one exhausts the pool — see the note
    // in `segments_from`.
    let entries = <SqliteStore as LogStore<
        KeysOperation,
        VerifyingKey,
        LogId,
        SeqNum,
        Hash,
    >>::get_log_entries(store, subject, &CONTROL_LOG_ID, after, None)
    .await?;

    let operations: Vec<KeysOperation> =
        entries.into_iter().flatten().map(|(op, _encoded_header)| op).collect();

    let mut out = Vec::with_capacity(operations.len());
    for (i, operation) in operations.iter().enumerate() {
        // A run that starts at the beginning of the log must start at seq 0
        // with nothing behind it; one that starts mid-log is trusted to begin
        // where the caller said, and its own links are still checked.
        let linked = match i {
            0 if after.is_none() => {
                operation.header.seq_num == 0 && operation.header.backlink.is_none()
            }
            0 => true,
            _ => p2panda_core::validate_backlink(&operations[i - 1].header, &operation.header)
                .is_ok(),
        };
        if !linked {
            return Err(Error::Forged(format!(
                "control log does not chain at seq {}",
                operation.header.seq_num
            )));
        }
        out.push(open_control(operation.clone(), subject)?);
    }
    Ok(out)
}
