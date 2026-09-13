package nz.diaswarm.jni

/**
 * The JNI contract over `crates/diaswarm-android`.
 *
 * IT LIVES HERE, BESIDE THE RUST, AND NOT IN THE AAPS TREE YET. The symbol
 * names in the native library are derived from this package and class name, so
 * renaming either half silently breaks the other at load time rather than at
 * compile time. Keeping the two in one repository means a rename is one commit
 * that fails loudly instead of two that fail in the field.
 *
 * NOTHING WITH A DECISION IN IT BELONGS ON THIS SIDE. The record shape, the
 * canonical encoding, deduplication and epoch arithmetic are all in
 * `diaswarm-core`, which is asserted byte-identical to `tools/canon.py` over the
 * whole reference snapshot. What Kotlin owns is what Rust cannot see: the typed
 * `DataPair`s from `DataSyncSelector`, the `isValid` check — the sync queue does
 * not filter on it, and xdrip never checks it either — and this plugin's own
 * high-water marks.
 */
object SwarmNative {

    /** Must match the spec version the plugin was written against. See [check]. */
    /**
     * Hand Android's `Context` to the network stack, once, before anything else.
     *
     * iroh watches for network changes through Android's ConnectivityManager;
     * without a Context it panics inside a tokio task, the task vanishes, and
     * the swarm keeps running with no idea that wifi dropped. Pass the
     * APPLICATION context — an Activity would be leaked for the life of the
     * process, because the native side keeps the reference for ever.
     */
    external fun initAndroid(context: android.content.Context)

    external fun specVersion(): Int

    /**
     * Which epoch a timestamp falls in, at a given fixed offset.
     *
     * Epochs are days cut at a fixed phase, not UTC days. At UTC+12 a UTC epoch
     * runs local noon to noon, so one local day splits across two — and a
     * revocation waiting for the boundary lands worst in the middle of the
     * waking day. The offset is a constant, not local time: a record's epoch
     * must not depend on where the phone was when it was written.
     */
    external fun epochOf(t: Long, offsetMs: Long): Long

    /** The stream header (spec §5.2), as a canonical JSON line. */
    external fun header(offsetMs: Long): String

    /** Canonicalise one record. Empty string if the input was not a record. */
    external fun canonicalLine(json: String): String

    /**
     * A deduplicating emitter, held across calls.
     *
     * The AAPS sync queue re-emits the current record on every edit, so
     * deduplication needs state that outlives a single record. The handle is a
     * raw pointer: call [emitterFree] exactly once, and do not rely on a
     * finaliser — leaking is better than freeing native state on a GC thread in
     * a process that has to keep dosing.
     */
    /**
     * A new emitter, resuming from [lastCgmBucket] (-1 for "nothing yet").
     *
     * See [app.aaps.plugins.sync.swarm.keys.SwarmLongKey.CgmBucketHighWater]:
     * the mark has to be carried across passes or CGM thinning never happens
     * live, because readings arrive one per minute in separate passes.
     */
    external fun emitterNew(lastCgmBucket: Long): Long
    external fun emitterFree(handle: Long)

    /** The canonical line to publish, or empty if already emitted. */
    external fun emitterAccept(handle: Long, json: String): String

    /**
     * Genuine edits seen — a record differing from one already emitted at the
     * same instant and kind.
     *
     * Counted, not handled. spec §7 reserves an `amend` kind and says to measure
     * before designing one: the reference snapshot showed roughly fifteen a year,
     * which is too few to guess a mechanism from. This counter is that
     * measurement, and it wants reading before anyone designs the mechanism.
     */
    external fun emitterAmendments(handle: Long): Long

    /** The newest CGM bucket emitted, to persist. -1 if none. */
    external fun emitterLastCgmBucket(handle: Long): Long

    /** How many CGM readings were thinned this run (spec §3.3). */
    external fun emitterThinned(handle: Long): Long

    /**
     * Seal one epoch of canonical NDJSON into the vault on disk.
     *
     * Creates the vault and the subject identity on first use. Returns the
     * number of records sealed, or a negative code — `-1` bad arguments, `-2`
     * identity unavailable, `-3` vault unavailable, `-4` seal failed. It does
     * not throw: an exception crossing JNI on a background thread inside a
     * process that has to keep dosing is a crash, not a diagnostic.
     */
    external fun vaultSeal(
        vaultPath: String,
        identityPath: String,
        epoch: Long,
        offsetMs: Long,
        ndjson: String
    ): Long

    /**
     * The subject's public key, to hand to someone you are granting.
     *
     * Creates the identity on first use. That file is the whole of what being
     * this subject means: lose it and every future grant is lost with it,
     * because the epoch keys it protects cannot be re-derived.
     */
    external fun vaultSubject(identityPath: String): String

    /** A one-line summary of the vault, for a log line or a status row. */
    external fun vaultStatus(vaultPath: String): String

    /**
     * Grant a reader everything, and publish their wraps. Returns wraps written,
     * or a negative code.
     */
    /**
     * Bring every granted reader's wraps up to date; returns how many were
     * written, or a negative code.
     *
     * Sealing keeps them current by itself, so on a healthy vault this is 0.
     * It is here for the vault that is not: one written by a build that wrapped
     * only at grant time, leaving followers with segments they cannot open and
     * nothing anywhere saying so.
     */
    external fun vaultRewrap(vaultPath: String): Long

    /** Who has been granted, one per line as `reader<TAB>purpose`. */
    external fun vaultReaders(vaultPath: String): String

    /**
     * The one string a subject hands to someone they want to share with.
     *
     * Built in Rust so the phone and the CLI cannot drift into two formats
     * that look alike and are not. Empty if any part is malformed.
     */
    external fun inviteFor(subject: String, endpoint: String, purpose: String, keys: String): String

    /** Read one back as `subject<TAB>endpoint<TAB>purpose`; empty if not valid. */
    external fun inviteParse(text: String): String

    // --- being in the pool -------------------------------------------------

    /**
     * Join the pool: find peers, work out this phone's share, hold it.
     *
     * Uses the same node key file as before, so the phone keeps the endpoint id
     * it already had — everything that ranks peers ranks them by it, and a new
     * key would read as one peer leaving and another arriving.
     *
     * Returns a handle, or 0 on failure.
     */
    external fun swarmJoin(storePath: String, nodeKeyPath: String, keysPath: String): Long

    /** This phone's id in the pool. Empty on a bad handle. */
    external fun swarmNodeId(handle: Long): String

    /**
     * One pass: say what we hold, hear what we should, take on up to
     * [maxAdopt] of them.
     *
     * Bounded because this runs on a phone — a peer joining a large pool would
     * otherwise try to pull its whole share at once, over mobile data, in a
     * worker with a deadline.
     *
     * Returns `pool<TAB>buckets<TAB>held<TAB>wanted<TAB>adopted`, or empty.
     */
    external fun swarmTick(handle: Long, maxAdopt: Long): String

    /** Leave the pool and release the handle. Idempotent on 0. */
    external fun swarmLeave(handle: Long)

    // --- following someone else -------------------------------------------

    /** Keep a copy of whoever sent this invite. 1 changed, 0 already known, <0 failed. */
    external fun netFollow(storePath: String, inviteText: String): Long

    /**
     * Bring every followed subject up to date. Returns how many were reached.
     *
     * Pass the serving handle when this phone is serving, so it fetches through
     * the endpoint it answers on. Peers remember whoever fetched from them and
     * pass that on; fetching from a throwaway endpoint hands out an address
     * that stops existing when the sync ends. 0 means "not serving".
     */
    external fun netRefresh(servingHandle: Long, storePath: String): Long

    /**
     * Accept a pushed invite for [seconds], because this phone just put its own
     * code on screen for somebody to scan.
     *
     * Without this window, anyone who knows this node id could make the phone
     * carry their ciphertext — and node ids are announced in the pool.
     */
    external fun swarmExpectOffer(handle: Long, seconds: Long)

    /**
     * Hand our invite to somebody whose code we just scanned, so they never
     * have to scan one back. 1 taken, 0 declined (their window was shut), <0
     * could not be delivered.
     */
    external fun swarmOffer(
        handle: Long,
        theirEndpoint: String,
        theirRelay: String,
        ourInvite: String
    ): Long

    /** What this phone follows: `subject<TAB>purpose<TAB>reached` per line. */
    external fun netFollowing(storePath: String): String

    /** Latest openable reading for a followed subject, as `mgdl<TAB>millis`. */
    external fun netLatest(
        storePath: String,
        subject: String,
        identityPath: String,
        purpose: String
    ): String

    /**
     * Every openable glucose reading for a followed subject after [sinceMs],
     * as `millis<TAB>mgdl<TAB>trend<TAB>src` lines, oldest first.
     *
     * [sinceMs] is EXCLUSIVE, so a caller can pass back the last timestamp it
     * stored. [limit] bounds the string that crosses JNI; getting exactly
     * [limit] lines means there is more, and the caller should ask again from
     * the last timestamp it saw. `trend` and `src` are enum NAMES, matching
     * what [SwarmRecords] wrote, and are empty when the sensor reported none.
     */
    external fun netGlucose(
        storePath: String,
        subject: String,
        identityPath: String,
        purpose: String,
        sinceMs: Long,
        limit: Int
    ): String

    /**
     * Every openable treatment for a followed subject after [sinceMs], as
     * `kind<TAB>millis<TAB>value<TAB>dur<TAB>flag` lines, oldest first.
     *
     * **THESE WERE ALWAYS BEING SENT.** The emitter drains `bolus`, `carb`,
     * `tbr` and `extbolus` alongside `cgm`; [netGlucose] filters to `cgm` and
     * discards the rest a line after decrypting them. This returns what was
     * already on the phone — no protocol change, no new grant.
     *
     * **PREDICTIONS ARE NOT HERE AND ARE NOT COMING.** `deviceStatus` and
     * `apsResults` are excluded at the emit boundary, so the loop's eventual-BG
     * and its own IOB are never published. Drawing them would mean computing
     * them, which is a different decision.
     *
     * `value` and `dur` mean different things per kind: bolus is units with no
     * duration; carb is grams; tbr is a rate that is U/h when `flag` is `abs`
     * and a percentage otherwise; extbolus is units. `dur` is milliseconds.
     */
    external fun netTreatments(
        storePath: String,
        subject: String,
        identityPath: String,
        purpose: String,
        sinceMs: Long,
        limit: Int
    ): String

    /**
     * The newest profile a followed subject has published, as canonical JSON,
     * or empty.
     *
     * Blocks are already mg/dL — the native side normalises on the way in, so
     * a consumer never has to know which unit the subject's phone was set to.
     */
    external fun netProfile(
        storePath: String,
        subject: String,
        identityPath: String,
        purpose: String
    ): String

    external fun vaultGrant(
        vaultPath: String,
        identityPath: String,
        readerPubHex: String,
        purpose: String
    ): Long

    /** Withdraw, immediately. Returns the segment it takes effect from. */
    external fun vaultRevoke(
        vaultPath: String,
        identityPath: String,
        readerPubHex: String,
        purpose: String
    ): Long

    // The bare-endpoint serving surface that used to live here is gone. Joining
    // the pool serves what we hold, on one endpoint, under one identity — and
    // two native handle types reachable from Kotlin is a crash waiting for
    // whoever passes the wrong one.


    // -----------------------------------------------------------------------
    // The spaces vault (D20/D21) — shadow only, see SwarmBooleanKey
    // -----------------------------------------------------------------------

    /** Open or create the spaces vault under [dir]. Returns a handle, or 0. */
    external fun spacesOpen(dir: String, offsetMs: Long): Long

    /** Close it. Safe with 0. */
    external fun spacesClose(handle: Long)

    /** The subject's public key, hex. Empty on failure. */
    external fun spacesSubject(handle: Long): String

    /** Seal canonical records. Returns how many, or < 0. */
    external fun spacesSeal(handle: Long, ndjson: String): Long

    /**
     * Grant a reader. [history] decides whether the grant reaches backwards:
     * true gives them everything ever sealed, false only what comes next.
     */
    external fun spacesGrant(handle: Long, readerHex: String, history: Boolean): Long

    /** Withdraw a reader, from the next thing sealed onward. */
    external fun spacesRevoke(handle: Long, readerHex: String): Long

    /** One line: windows and readers. */
    external fun spacesStatus(handle: Long): String


    // -----------------------------------------------------------------------
    // The keys vault (D26) — what shadow mode actually shadows
    // -----------------------------------------------------------------------
    //
    // The spaces entry points above stay because the code is still there and
    // still measured, but D26 decided against that message layer: it cannot
    // express a follower who reads only the last day. Shadowing it was
    // measuring the thing that is not going to ship.

    /**
     * Open or create the keys vault under [dir], signing as the identity at
     * [identityPath] — the same one the core vault's grant log is signed with,
     * so a subject is one subject whichever vault is asked.
     *
     * Returns a handle, or 0.
     */
    external fun keysOpen(pool: Long, dir: String, identityPath: String, offsetMs: Long): Long

    /** Close it. Safe with 0. */
    external fun keysClose(handle: Long)

    /**
     * The key this vault's logs are authored under, hex. Empty on failure.
     *
     * **NOT THE MEMBER TAG, AND NOT THE INVITE'S `subject` EITHER.** This is
     * the Ed25519 key `keysCarry`, the control log and the segment log are all
     * keyed by. The invite's `subject` is the core vault's X25519 key, and
     * neither can be derived from the other.
     */
    external fun keysSubject(handle: Long): String

    /**
     * Seal a batch into one epoch, read it back off disk, and report.
     *
     * **THE READ-BACK IS THE POINT.** A seal count is a statement about the
     * argument, not about the vault. This re-opens the segment afterwards and
     * checks every record handed in comes back out.
     *
     * Returns one line, always parseable, never throwing:
     * `ok epoch=20342 given=37 held=1586 missing=0 lost=0`, or `error <what>`.
     * `missing` is the number this whole feature exists to produce.
     */
    external fun keysSealChecked(handle: Long, epoch: Long, ndjson: String): String

    /**
     * This vault's identity as text, for the `keys` field of an invite.
     *
     * **BOTH KEYS, BECAUSE NEITHER IMPLIES THE OTHER.** The Ed25519 key its
     * logs are authored under — without which a follower cannot find anything
     * to fetch — and the key bundle a grant is agreed against. Empty on
     * failure.
     */
    external fun keysIdentity(handle: Long): String

    /**
     * Grant a reader from the bundle they published. Returns their tag, or
     * `error …`.
     *
     * The tag is the subject's private name for the relationship and belongs in
     * the subject's own book; the grant itself names nobody. The welcome is in
     * the control log before this returns — a grant that is not in the log has
     * not happened.
     */
    external fun keysGrant(handle: Long, readerBundle: String, purpose: String): String

    /**
     * Withdraw a reader by the tag [keysGrant] returned. 0 on success.
     *
     * Rotates the group secret, so it bites the day it happens in rather than
     * from midnight. Days already finished keep their own secret and stay
     * readable — a withdrawal, not a deletion.
     */
    external fun keysRevoke(handle: Long, tagHex: String): Long

    /**
     * Join a subject we have been granted access to, and read what they shared.
     *
     * **ONE CALL, BECAUSE A FOLLOWER WANTS RECORDS AND NOT A HANDLE.** Joining
     * is a one-off; a vault that is already welcomed skips straight to reading.
     * That is correctness rather than speed — joining replaces the group state,
     * so doing it on every refresh would throw away a secret bundle that took a
     * replication round trip to acquire.
     *
     * [handle] is this device's OWN keys vault: it supplies the identity each
     * subject granted against, and the store replication writes into. A joined
     * vault that minted its own identity would be a different member to the one
     * that was granted and would read nothing, without an error.
     *
     * Returns `ok <segments> <unreadable>` then a newline then NDJSON, or
     * `error <what>` — never a bare count, because a follower that read nothing
     * needs to know whether it was never granted, never replicated, or simply
     * has no days yet.
     */
    external fun keysFollowRead(
        handle: Long,
        joinedDir: String,
        subjectKeys: String,
        purpose: String,
        tailDays: Long,
        fromEpoch: Long
    ): String

    /**
     * Carry a subject's keys logs on the bucket topic they fall in.
     *
     * Idempotent — a pool pass calls it for everything this phone should hold,
     * and most of that is already known. Both logs in one call: a follower with
     * segments and no grants cannot open them, and one with grants and no
     * segments has nothing to open. 0 on success.
     */
    external fun keysCarry(poolHandle: Long, subjectHex: String): Long

    /**
     * Carry every keys log this phone should hold: its own, and each it
     * follows. Returns how many, or negative.
     *
     * **OUR OWN IS THE PUBLISHING HALF** — a subject that does not announce its
     * own bucket is one nobody can replicate from. A follow with no keys
     * identity is skipped rather than failed: somebody paired before the field
     * existed, or a subject with no keys vault, and neither is an error.
     */
    external fun keysCarryAll(poolHandle: Long, storePath: String, identityPath: String): Long

    /**
     * Take the handover queue, keep what verifies, and say what may be granted.
     *
     * **THE QUEUE IS WRITTEN BY STRANGERS.** `Request::Handover`'s handler
     * records claims without believing any of them — it answers anyone and
     * holds no key. This is the other half, where the subject's own secret
     * recomputes the proof; only a claim matching a reader already in the
     * private book survives.
     *
     * Returns `keys-identity<TAB>purpose` per verified claim, for the caller to
     * grant on the keys vault. A claim that does not verify is dropped and not
     * reported: saying which failed would answer "is this tag one you have
     * granted?" for anybody who asked.
     */
    /**
     * Offer our keys identity to a subject we already follow, and prove it is
     * ours (D27).
     *
     * 1 if the subject took it, 0 if not — which includes a subject too old to
     * understand the request, and is not an error: the reader goes on reading
     * the vault it already reads.
     */
    external fun netHandOver(
        poolHandle: Long,
        storePath: String,
        identityPath: String,
        subjectHex: String,
        keysIdentity: String
    ): Long

    external fun vaultAcceptHandovers(
        storePath: String,
        vaultPath: String,
        identityPath: String
    ): String

    /**
     * A followed subject's readings out of the keys vault, in [netGlucose]'s
     * shape: `millis<TAB>mgdl<TAB>trend<TAB>src`, oldest first.
     *
     * **THE SAME ROWS, SO THE CHART DOES NOT CHANGE.** Returning anything else
     * would mean rewriting the screen to find out whether the vault works, and
     * the screen is not what is being tested. Empty when nothing can be
     * opened — the same answer [netGlucose] gives.
     */
    external fun keysGlucose(
        handle: Long,
        joinedDir: String,
        subjectKeys: String,
        purpose: String,
        sinceMs: Long,
        limit: Long
    ): String

    /** The spec version this Kotlin was written against. */
    const val EXPECTED_SPEC_VERSION = 3

    /**
     * Load the native library and refuse to run against a mismatched one.
     *
     * A native library that disagrees with the Kotlin about the spec version is
     * how a stream ends up conforming to a specification nobody believes it
     * conforms to. Fail at load, loudly, rather than publishing.
     */
    fun check() {
        System.loadLibrary("diaswarm_android")
        val actual = specVersion()
        require(actual == EXPECTED_SPEC_VERSION) {
            "native library implements spec v$actual, plugin expects v$EXPECTED_SPEC_VERSION"
        }
    }
}
