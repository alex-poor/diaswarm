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

use futures_util::StreamExt;
use p2panda_core::{AnyOperation, Body, Hash, Header, SigningKey, VerifyingKey};
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
/// A reader finds a day's data by walking [`LOG_ID`] back from the tip and
/// reading each entry's epoch ([`segments_covering`]). That only works if every
/// entry in the log is a segment. Interleaved grants carry no epoch, would
/// each cost an Ed25519 verification for nothing, and would make the stopping
/// rule — "stop once something older than the wanted epochs appears" — reason
/// about entries that are not data.
///
/// **THIS USED TO GIVE A DIFFERENT REASON, AND IT STOPPED BEING TRUE.** It said
/// every entry in log 0 was one day's segment, so [`segments_tail`]'s "last N
/// entries" meant "last N days". Since publishing moved to deltas
/// (`seal_delta`, one segment per flush, about once a minute) a day is many
/// entries; see `follow.rs`. The separate log was right for both reasons.
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

/// A control operation that is NOT a group message: it records where an epoch
/// begins in the SEGMENT log, the index a windowing follower reads.
///
/// **IT IS A `v`, NOT A NEW FIELD, SO THE HEADER WIRE FORMAT DOES NOT CHANGE.**
/// `ControlArgs` stays `{ v: u8 }`; an anchor is just a control op whose `v` says
/// "this body is an [`Anchor`], not a [`Message`]". A reader that does not know
/// this kind skips it (see [`control_from`]) instead of failing to decode it as a
/// message — which is why the skip must ship to followers BEFORE a publisher ever
/// emits one.
pub const ANCHOR_V1: u8 = 2;

/// One entry of the epoch→segment-seq index that lets a follower window.
///
/// **THE WHOLE POINT: WINDOW WITHOUT THE SEGMENT LOG.** A follower that wants only
/// "the last N epochs" needs the segment sequence number each epoch begins at — but
/// that lives in the segment headers, which is exactly what it is trying NOT to
/// mirror. So the publisher copies the boundary into the CONTROL log (tiny, never
/// windowed, always fully synced) as one `Anchor` per epoch: the first segment of
/// `epoch` sits at segment-log sequence `seq`. The follower reads these
/// ([`anchors_from`]), picks the floor for its window ([`floor_for_epochs`]) and
/// advertises that height — see `WindowedStore` in `ingestprobe`.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Anchor {
    pub epoch: i64,
    pub seq: SeqNum,
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
            AnyOperation,
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
        // MIGRATION (#1612): `insert_operation` -> `insert_operation_tx`. The
        // write side still takes a TYPED `Operation<E>` (only the read side went
        // untyped to `AnyOperation`), so the operation is passed as-is.
        store.insert_operation_tx(&operation.hash, &operation, &LOG_ID).await?;
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
/// the skipping.
///
/// **"THE LAST N ENTRIES" IS NOT "THE LAST N DAYS" ANY MORE.** That held when
/// each entry was a whole day; a subject now publishes a delta per flush, so N
/// entries can be N minutes of today. Readers use [`segments_covering`]. This
/// stays for callers that really do want entries by count — the replication
/// tests and `twokeys` — and should not be used to mean days.
pub async fn segments_tail(
    store: &SqliteStore,
    author: &VerifyingKey,
    count: u64,
) -> Result<Vec<Segment>, Error> {
    let heights = <SqliteStore as LogStore<
        AnyOperation,
        VerifyingKey,
        LogId,
        SeqNum,
        Hash,
    >>::get_log_heights(store, author, &[LOG_ID])
    .await?;
    let tip = heights.and_then(|h| h.get(&LOG_ID).copied()).unwrap_or(0);
    let after = (tip as u64).saturating_sub(count.saturating_sub(1));
    let after = if after == 0 { None } else { Some((after - 1) as SeqNum) };

    let entries = read_ops(store, author, &LOG_ID, after, None).await?;
    collect(entries)
}

/// Prune the segment log down to its most recent `keep` operations (D35).
///
/// **THIS IS HOW A NEW FOLLOWER STOPS HAVING TO MIRROR A WHOLE HISTORY.** A fresh
/// follower syncs every operation a subject holds; against a real-history subject
/// that reached ~3.4 GB and crashed the app with a native OOM. p2panda is built
/// assuming applications prune, and we never did. Deleting older segments here —
/// from the SHARED vault, which is not where the subject's own records live (those
/// are in AAPS) — bounds what any fresh follower syncs to a recent window, because
/// a peer only serves what it still holds.
///
/// By count, because the store prunes by sequence number and segments are
/// contiguous from zero; the caller picks `keep` to cover the retention window it
/// wants. Only [`LOG_ID`] (segments) is touched — never [`CONTROL_LOG_ID`], whose
/// grants and epoch keys a follower needs to open anything and which is tiny.
/// Returns how many operations were pruned.
pub async fn prune_segments(
    store: &SqliteStore,
    author: &VerifyingKey,
    keep: u64,
) -> Result<u64, Error> {
    let size = <SqliteStore as LogStore<
        AnyOperation,
        VerifyingKey,
        LogId,
        SeqNum,
        Hash,
    >>::get_log_size(store, author, &LOG_ID, None, None)
    .await?;
    let count = size.map(|(ops, _bytes)| ops as u64).unwrap_or(0);
    if count <= keep {
        return Ok(0);
    }
    // Segments are seq 0..count-1, so keeping the last `keep` means pruning
    // everything before seq (count - keep) — proven by `ingestprobe`.
    let until = (count - keep) as SeqNum;
    let pruned = tx!(store, {
        <SqliteStore as LogStore<
            AnyOperation,
            VerifyingKey,
            LogId,
            SeqNum,
            Hash,
        >>::prune_entries_tx(store, author, &LOG_ID, &until)
        .await?
    });
    Ok(pruned)
}

/// What [`segments_covering`] was last asked, so the readers that ask it four
/// times a refresh pay once.
#[derive(PartialEq, Eq)]
struct CacheKey {
    author: VerifyingKey,
    /// The log height. This is what makes the entry self-invalidating.
    tip: u64,
    from_epoch: i64,
    tail_days: u64,
}

/// **BOUNDED TWICE, ON PURPOSE.** An unbounded cache is how this project has
/// spent two nights already — `pending_open_paths` growing to gigabytes, and a
/// `StringBuilder` that accumulated every record ever drained because the
/// feature it belonged to was switched off.
///
/// A count alone is not a bound here. A `tail_days == 0` entry is the whole
/// log, which is megabytes for a subject with a year of history and grows
/// every day, so "four entries" could mean forty megabytes without anything
/// looking wrong. The byte cap is the real limit; the count stops a lot of
/// tiny windows accumulating.
///
/// Entries hold ciphertext, not plaintext or keys: the same bytes already on
/// disk, for a window a caller is actively reading. Overshooting the cap costs
/// a re-read, which is the thing this exists to avoid but is never wrong.
const CACHE_ENTRIES: usize = 8;
const CACHE_BYTES: usize = 24 * 1024 * 1024;

fn entry_bytes(segments: &[Segment]) -> usize {
    segments.iter().map(|s| s.ciphertext.len()).sum()
}

#[allow(clippy::type_complexity)]
static SEGMENT_CACHE: std::sync::Mutex<Vec<(CacheKey, std::sync::Arc<Vec<Segment>>)>> =
    std::sync::Mutex::new(Vec::new());

fn cache_get(key: &CacheKey) -> Option<Vec<Segment>> {
    // A poisoned lock is not a reason to fail a read — the cache is an
    // optimisation and the log is the truth. Clear it and carry on.
    let mut guard = match SEGMENT_CACHE.lock() {
        Ok(g) => g,
        Err(poisoned) => {
            let mut g = poisoned.into_inner();
            g.clear();
            g
        }
    };
    let found = guard.iter().position(|(k, _)| k == key)?;
    // Most-recently-used to the back, so eviction takes the coldest.
    let entry = guard.remove(found);
    let out = entry.1.as_ref().clone();
    guard.push(entry);
    Some(out)
}

fn cache_put(key: CacheKey, value: &[Segment]) {
    let mut guard = match SEGMENT_CACHE.lock() {
        Ok(g) => g,
        Err(poisoned) => {
            let mut g = poisoned.into_inner();
            g.clear();
            g
        }
    };
    guard.retain(|(k, _)| k != &key);
    // A single window larger than the whole budget is not worth evicting
    // everything else for, and caching it would break the bound on its own.
    if entry_bytes(value) > CACHE_BYTES {
        return;
    }
    guard.push((key, std::sync::Arc::new(value.to_vec())));
    while guard.len() > CACHE_ENTRIES
        || guard.iter().map(|(_, v)| entry_bytes(v)).sum::<usize>() > CACHE_BYTES
    {
        // Never evict the entry just inserted: the caller is using it now, and
        // dropping it would make a large window cost a re-read every time.
        if guard.len() <= 1 {
            break;
        }
        guard.remove(0);
    }
}

/// Forget everything cached. For tests that want a cold read, and for any
/// caller that has reason to believe the log was rewritten under it — pruning
/// removes entries without moving the tip, which is the one thing the tip
/// cannot notice.
pub fn forget_cached_segments() {
    if let Ok(mut g) = SEGMENT_CACHE.lock() {
        g.clear();
    }
}

/// The segments covering the newest `tail_days` epochs, and no more of the log
/// than that needs.
///
/// **WHY THIS EXISTS: `segments_tail` FETCHES 20,000 OPERATIONS TO USE ABOUT A
/// HUNDRED.** A follower wanting today asked for `tail_days * 320` entries
/// capped at 20,000, and `AnyHeader::decode` runs an Ed25519 verification on
/// every one of them before the caller throws away the epochs it did not want.
/// `follow.rs` predicted the bill in its own comment — "the thing to revisit
/// first if replication gets expensive" — and on 2026-09-17 it was paid: a
/// profile of the follower put 96% of its asymmetric crypto under
/// `Header<KeysArgs>::decode`, reached from the `keys*` readers, at 349 ms a
/// read over a real 9,564-operation log. Four of those per refresh is most of
/// a core.
///
/// So walk BACK from the tip in chunks and stop as soon as the wanted epochs
/// are whole, rather than guessing a width in operations and over-fetching to
/// be safe.
///
/// **THE STOPPING RULE IS "WE HAVE SEEN SOMETHING OLDER", NOT "WE HAVE ENOUGH
/// EPOCHS".** Deltas, not supersets, are what gets published — `seal_delta` is
/// what the publisher hands to [`publish`], and the merged day lives only on
/// the subject's own disk — so one epoch is spread across every flush that
/// touched it and a reader needs all of them. Stopping at the moment the
/// oldest wanted epoch first appears would silently truncate that epoch to its
/// newest few flushes. We are only done when an operation OLDER than the
/// oldest wanted epoch has been seen, which proves nothing further back belongs
/// to it.
///
/// Epochs are not assumed to be monotonic in seq_num: a re-drain can publish an
/// older epoch late. Walking back until something older appears tolerates that
/// where a binary search would not.
pub async fn segments_covering(
    store: &SqliteStore,
    author: &VerifyingKey,
    from_epoch: i64,
    tail_days: u64,
    chunk: u64,
) -> Result<Vec<Segment>, Error> {
    let chunk = chunk.max(1);

    let heights = <SqliteStore as LogStore<
        AnyOperation,
        VerifyingKey,
        LogId,
        SeqNum,
        Hash,
    >>::get_log_heights(store, author, &[LOG_ID])
    .await?;
    let tip = heights.and_then(|h| h.get(&LOG_ID).copied()).unwrap_or(0) as u64;

    // **FOUR READERS ASK THIS SAME QUESTION PER REFRESH.** The profile that
    // started this put 216 samples under `keysProfile`, 200 under
    // `keysTempTarget`, 45 under `keysGlucose` and 28 under `keysTreatments` —
    // four separate JNI reads, each re-fetching and re-verifying the same
    // window, because each wants one different field out of it.
    //
    // Keyed on the log TIP, so it cannot go stale: a tip that has not moved is
    // a log that has not been appended to, and the answer cannot have changed.
    // A new flush moves the tip and the entry is simply never hit again.
    let key = CacheKey { author: *author, tip, from_epoch, tail_days };
    if let Some(hit) = cache_get(&key) {
        return Ok(hit);
    }

    // **`tail_days == 0` MEANS "EVERY EPOCH", AND IT IS THE EXPENSIVE CALLER,
    // NOT THE CHEAP ONE.** This used to return here before the cache was even
    // consulted, which is how the first version of this fix measured a 96×
    // improvement on a bench and nothing at all on a phone: `keysProfile` and
    // `keysTempTarget` — 416 of the 496 samples that started this — both ask
    // for every epoch, because a profile is published only when it changes and
    // a tail window would report "no profile" for somebody whose settings are
    // simply stable. They took this early return and skipped both fixes.
    //
    // Reading the whole log is inherent to that question and is left alone.
    // Being asked it four times per refresh is not, which is what the cache
    // above is for: once per new flush instead of once per reader.
    if tail_days == 0 {
        let all = segments_from(store, author, from_epoch).await?;
        cache_put(key, &all);
        return Ok(all);
    }

    let mut collected: Vec<Segment> = Vec::new();
    // `until` is inclusive of the entry at that seq; `after` is exclusive. We
    // walk the window [lo, hi] downwards.
    let mut hi = tip;
    loop {
        let lo = hi.saturating_sub(chunk.saturating_sub(1));
        let after = if lo == 0 { None } else { Some((lo - 1) as SeqNum) };
        let entries = read_ops(store, author, &LOG_ID, after, Some(hi as SeqNum)).await?;
        let mut batch = collect(entries)?;
        batch.append(&mut collected);
        collected = batch;

        // The epochs we would keep if we stopped now: the newest `tail_days`
        // of those at or after `from_epoch`.
        let mut epochs: Vec<i64> =
            collected.iter().map(|s| s.epoch).filter(|e| *e >= from_epoch).collect();
        epochs.sort_unstable();
        epochs.dedup();
        let oldest_wanted = epochs.iter().rev().take(tail_days as usize).next_back().copied();

        let done = match oldest_wanted {
            // Something strictly older than the oldest epoch we want is in
            // hand, so that epoch cannot continue further back.
            Some(oldest) => {
                epochs.len() >= tail_days as usize
                    && collected.iter().any(|s| s.epoch < oldest)
            }
            // Nothing at or after `from_epoch` yet — keep walking back only
            // while there is log left, because there may be none at all.
            None => false,
        };
        if done || lo == 0 {
            break;
        }
        hi = lo - 1;
    }
    cache_put(key, &collected);
    Ok(collected)
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
    let entries = read_ops(store, author, &LOG_ID, None, None).await?;
    Ok(collect(entries)?.into_iter().filter(|s| s.epoch >= from_epoch).collect())
}

/// The current tips of both of a subject's logs: `(segment, control)`.
///
/// **A READER'S MEMO KEY NEEDS BOTH.** A live display that re-reads its window
/// on every refresh can skip the whole decrypt when nothing changed — but
/// "nothing changed" is not just "no new segment". A rotation or a fresh grant
/// lands in the *control* log and leaves the segment tip untouched, yet it
/// changes what the reader can open: the 2026-09-16 incident was exactly a
/// rotation that had not been noticed, a reader reporting "outside your window"
/// about data squarely inside it. So a cache that keys only on the segment tip
/// would serve a stale read straight past a rotation. Keying on both tips makes
/// a control-log change invalidate the read, which is correct and cheap — both
/// heights come from one indexed query.
pub async fn log_tips(store: &SqliteStore, author: &VerifyingKey) -> Result<(SeqNum, SeqNum), Error> {
    let heights = <SqliteStore as LogStore<
        AnyOperation,
        VerifyingKey,
        LogId,
        SeqNum,
        Hash,
    >>::get_log_heights(store, author, &LOG_IDS)
    .await?;
    let at = |id: LogId| heights.as_ref().and_then(|h| h.get(&id).copied()).unwrap_or(0);
    Ok((at(LOG_ID), at(CONTROL_LOG_ID)))
}

/// Only the segments appended *after* a sequence-number mark, plus the log's
/// current tip so the caller can persist a new mark.
///
/// **THIS IS THE INCREMENTAL READ A LIVE FOLLOWER WANTS.** [`read`] and
/// [`segments_covering`] answer "the last N days" and re-fetch, re-verify and
/// re-decrypt that whole window on every call — which is cheap on a laptop and,
/// measured on a phone, is 64% of its native cycles and 28% of its allocator in
/// the follow path. A display that already showed everything up to seq `m` does
/// not need the window again; it needs whatever arrived since. `after = Some(m)`
/// returns exactly the entries with `seq > m` (the store does the skipping, the
/// same exclusive `after` [`segments_tail`] uses), so a steady poll fetches,
/// verifies and decrypts only the new operations.
///
/// Returns `(segments, tip)`. `tip` is the newest sequence number now in the
/// log; the caller passes it back as `after` next time. `after = None` is the
/// first read and returns the whole log, with the tip to start marking from.
///
/// 🔴 **A RE-DRAIN LANDS HERE TOO.** A subject that re-publishes records it
/// already sent gives them new sequence numbers, so they are "after" the mark
/// and come back again. That is correct — they are new log entries — but it
/// means the caller must still dedup what it accumulates, exactly as [`read`]
/// does within one window. The mark bounds the common case (append-only); it is
/// not a promise that a record is delivered once.
pub async fn segments_after(
    store: &SqliteStore,
    author: &VerifyingKey,
    after: Option<SeqNum>,
) -> Result<(Vec<Segment>, SeqNum), Error> {
    let heights = <SqliteStore as LogStore<
        AnyOperation,
        VerifyingKey,
        LogId,
        SeqNum,
        Hash,
    >>::get_log_heights(store, author, &[LOG_ID])
    .await?;
    let tip = heights.and_then(|h| h.get(&LOG_ID).copied()).unwrap_or(0);

    // No `tx!` — this is a plain read; see the note on `segments_from`.
    let entries = read_ops(store, author, &LOG_ID, after, None).await?;
    Ok((collect(entries)?, tip))
}

/// Read a log's entries and decode each back into a typed [`KeysOperation`].
///
/// **MIGRATION (#1612 p2panda-store rewrite).** `LogStore` no longer knows the
/// extension type: `get_log_entries` (async, `Vec<(Operation<E>, _)>`) became
/// `log_entries` (a lazy `futures` stream of `LogEntry<AnyOperation, _>`). The
/// header's `KeysArgs` are decoded here, at the store boundary, with
/// `AnyOperation::try_into`, so the rest of this module stays typed exactly as
/// before. This conversion is a CBOR decode of the extensions only — it does
/// NOT re-verify the signature; that stays [`p2panda_core::validate_operation`]
/// where it was, so the Ed25519 cost profiled in `segments_covering` is
/// unchanged by the migration.
pub(crate) async fn read_ops(
    store: &SqliteStore,
    author: &VerifyingKey,
    log_id: &LogId,
    after: Option<SeqNum>,
    until: Option<SeqNum>,
) -> Result<Vec<KeysOperation>, Error> {
    let mut stream = <SqliteStore as LogStore<
        AnyOperation,
        VerifyingKey,
        LogId,
        SeqNum,
        Hash,
    >>::log_entries(store, author, log_id, after, until)?;
    let mut ops = Vec::new();
    while let Some(item) = stream.next().await {
        let item = item?;
        let op: KeysOperation = item.entry.try_into().map_err(|e| {
            Error::Forged(format!("a stored header will not decode: {e}"))
        })?;
        ops.push(op);
    }
    Ok(ops)
}

/// Turn a log's operations into the segments they carry.
///
/// Each operation's ciphertext is its **body**; the header names the epoch,
/// secret and nonce ([`SegmentArgs`]). A control operation sitting in the
/// segment log is a forgery signal, not a segment with odd fields.
fn collect(ops: Vec<KeysOperation>) -> Result<Vec<Segment>, Error> {
    let mut out = Vec::new();
    for op in ops {
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
            AnyOperation,
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
        store.insert_operation_tx(&operation.hash, &operation, &CONTROL_LOG_ID).await?;
        operation
    });
    Ok(operation)
}

/// The sequence number the next segment will take — a fresh log's first op is seq
/// 0, not 1, which is the distinction a plain tip height (`log_tips`) loses.
///
/// This is what an epoch anchor records: at rotation, before the epoch's first
/// segment is sealed, this is the seq that segment will land at. Computed exactly
/// as [`publish`] computes its own seq, so the two cannot disagree.
pub async fn next_segment_seq(
    store: &SqliteStore,
    author: &VerifyingKey,
) -> Result<SeqNum, Error> {
    // A plain read (like `log_tips`), NOT `get_latest_entry_tx` — that one must run
    // inside a transaction, and this is a standalone read. `get_log_heights`
    // returns None for an empty log (→ next seq 0) and the tip otherwise (→ tip+1),
    // which is exactly the empty-vs-seq-0 distinction a tip height alone would lose.
    let heights = <SqliteStore as LogStore<
        AnyOperation,
        VerifyingKey,
        LogId,
        SeqNum,
        Hash,
    >>::get_log_heights(store, author, &[LOG_ID])
    .await?;
    Ok(match heights.and_then(|h| h.get(&LOG_ID).copied()) {
        Some(tip) => tip + 1,
        None => 0,
    })
}

/// Append an epoch [`Anchor`] to the control log, signed (the window index).
///
/// Like [`publish_control`] but the body is an `Anchor`, not a [`Message`], and the
/// header is tagged [`ANCHOR_V1`] so the message reader ([`control_from`]) skips it.
/// There is no `sender` check — an anchor is not a group message; its authenticity
/// is the signed header, the same thing that authenticates any control op.
pub async fn publish_anchor(
    store: &SqliteStore,
    signing_key: &SigningKey,
    anchor: &Anchor,
) -> Result<KeysOperation, Error> {
    let payload =
        p2panda_core::cbor::encode_cbor(anchor).map_err(|e| Error::Encode(e.to_string()))?;

    // ONE TRANSACTION, for the reason `publish` gives: read-then-append.
    let operation = tx!(store, {
        let (seq_num, backlink) = <SqliteStore as LogStore<
            AnyOperation,
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
            .build(signing_key, KeysArgs::Control(ControlArgs { v: ANCHOR_V1 }));
        let operation = KeysOperation::from_parts(header, Some(Body::from_bytes(payload)));
        store.insert_operation_tx(&operation.hash, &operation, &CONTROL_LOG_ID).await?;
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
    let operations = read_ops(store, subject, &CONTROL_LOG_ID, after, None).await?;

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
        // **SKIP CONTROL OPS THAT ARE NOT MESSAGES.** The chain is still checked
        // above, so a non-message in the log cannot hide a break — but an
        // [`ANCHOR_V1`] index entry (or any future kind) is not something the group
        // applies, and decoding its body as a `Message` would fail the whole read.
        // This skip is the forward-compatibility that has to reach followers before
        // a publisher emits the first anchor.
        if let KeysArgs::Control(c) = &operation.header.extensions {
            if c.v != CONTROL_V1 {
                continue;
            }
        }
        out.push(open_control(operation.clone(), subject)?);
    }
    Ok(out)
}

/// Every epoch [`Anchor`] in `subject`'s control log, authenticated and sorted by
/// epoch.
///
/// The control log is always fully synced, so this is complete even for a follower
/// that holds none of the segment log — which is the whole reason the index lives
/// here. Each anchor is a signed control op; its authenticity is checked the same
/// way [`open_control`] checks a message, minus the `sender` field a message has
/// and an anchor does not.
pub async fn anchors_from(
    store: &SqliteStore,
    subject: &VerifyingKey,
) -> Result<Vec<Anchor>, Error> {
    let operations = read_ops(store, subject, &CONTROL_LOG_ID, None, None).await?;

    let mut out = Vec::new();
    for operation in &operations {
        match &operation.header.extensions {
            KeysArgs::Control(c) if c.v == ANCHOR_V1 => {}
            _ => continue,
        }
        if operation.header.verifying_key != *subject {
            return Err(Error::Forged(format!(
                "an anchor at seq {} is signed by the wrong key",
                operation.header.seq_num
            )));
        }
        let body = operation
            .body
            .as_ref()
            .ok_or_else(|| Error::Forged("an anchor arrived with no body".to_string()))?
            .to_bytes();
        p2panda_core::validate_operation(operation).map_err(|e| Error::Forged(e.to_string()))?;
        let anchor: Anchor = p2panda_core::cbor::decode_cbor(&body[..])
            .map_err(|e| Error::Encode(format!("anchor will not decode: {e}")))?;
        out.push(anchor);
    }
    out.sort_by_key(|a| a.epoch);
    Ok(out)
}

/// The segment-log height a follower advertises to sync only the last
/// `keep_epochs` epochs — or `None` to sync the whole log.
///
/// Reads the index ([`anchors_from`]) and returns `Some(seq - 1)` for the first
/// segment of the earliest epoch inside the window, so a `Have` at that height
/// makes a peer send that segment and everything after it and nothing before.
///
/// **BY EPOCH VALUE, NOT ANCHOR COUNT**, because "the last N days" is a span of
/// time: a day with no readings seals no segment and writes no anchor, and it must
/// still count against the window. So the cutoff is `latest_epoch - (keep - 1)` and
/// the floor is the first anchor at or after it. Returns `None` when the window
/// already covers everything the index knows (keep spans the whole history, or the
/// earliest kept segment is seq 0) — there is nothing to trim.
pub async fn floor_for_epochs(
    store: &SqliteStore,
    subject: &VerifyingKey,
    keep_epochs: u32,
) -> Result<Option<SeqNum>, Error> {
    if keep_epochs == 0 {
        return Ok(None);
    }
    let anchors = anchors_from(store, subject).await?;
    let (Some(earliest), Some(latest)) = (anchors.first(), anchors.last()) else {
        return Ok(None);
    };
    let cutoff = latest.epoch - (keep_epochs as i64 - 1);
    if cutoff <= earliest.epoch {
        return Ok(None); // the window reaches the start of the index: keep it all
    }
    // `latest` itself satisfies `epoch >= cutoff`, so this always finds one.
    let start = anchors.iter().find(|a| a.epoch >= cutoff).expect("latest qualifies");
    Ok(if start.seq == 0 { None } else { Some(start.seq - 1) })
}
