package app.aaps.plugins.sync.swarm

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
    external fun emitterNew(): Long
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
    external fun inviteFor(subject: String, endpoint: String, purpose: String): String

    /** Read one back as `subject<TAB>endpoint<TAB>purpose`; empty if not valid. */
    external fun inviteParse(text: String): String

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

    /**
     * Start serving this vault to peers. Returns a handle, or 0.
     *
     * **The point the phone stops being alone.** Until now the ciphertext sat on
     * one device; from here a peer can hold it, and §7.4's price list starts
     * applying: what leaves is permanent.
     */
    external fun netStart(vaultPath: String, nodeKeyPath: String): Long

    /** The endpoint id a peer dials. Empty if not serving. */
    external fun netEndpointId(handle: Long): String

    /** Stop serving and release the handle. A no-op on 0. */
    external fun netStop(handle: Long)

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
