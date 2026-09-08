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

    /** Which UTC-day epoch a timestamp falls in — the unit of key custody. */
    external fun epochOf(t: Long): Long

    /** The stream header (spec §5.2), as a canonical JSON line. */
    external fun header(): String

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

    /** The spec version this Kotlin was written against. */
    const val EXPECTED_SPEC_VERSION = 1

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
