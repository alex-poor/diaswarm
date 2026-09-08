package app.aaps.plugins.sync.swarm

import app.aaps.core.interfaces.db.PersistenceLayer
import app.aaps.core.interfaces.logging.AAPSLogger
import app.aaps.core.interfaces.logging.LTag
import app.aaps.core.interfaces.sync.DataSyncSelector
import app.aaps.core.keys.interfaces.Preferences
import app.aaps.plugins.sync.swarm.keys.SwarmLongKey
import javax.inject.Inject
import javax.inject.Singleton

/**
 * Drains the AAPS sync queue outward into canonical records.
 *
 * READ-ONLY, STRUCTURALLY (decision D7). A `DataSyncSelector` drains a queue
 * outward; nothing in this shape can write back into the loop, and nothing here
 * may acquire the ability. Anything that can influence dosing is inside the
 * medical device's blast radius and inherits its whole risk posture.
 *
 * WHAT THIS HAS TO DO THAT THE SNAPSHOT TOOL DOES NOT.
 *
 *  1. CHECK `isValid` ITSELF. The queue does not filter on it —
 *     `getNextModifiedOrNewAfter` is `SELECT * FROM t WHERE id > :id ORDER BY id
 *     ASC LIMIT 1`, no predicate — and xDrip never checks it either. A retracted
 *     record reaching a consumer is a record the user withdrew.
 *
 *  2. DEDUPLICATE RATHER THAN DROP VERSION ROWS. `spec/records.md` §3 drops
 *     rows with a `referenceId` because a snapshot shows only final state. The
 *     queue does the opposite: it walks every row including version rows and
 *     resolves each to the CURRENT record, so the same logical record arrives
 *     again on every edit. Measured on the reference snapshot: 12,874 of 12,876
 *     version rows carry no clinical change at all.
 *
 *  3. COUNT WHAT IT CANNOT SETTLE. A genuine edit arriving after its epoch was
 *     sealed cannot be un-published (§7.4). There were two in 48.5 days. spec §7
 *     reserves an `amend` kind and says to measure before designing one, so this
 *     counts them into [SwarmLongKey.AmendmentsSeen] and does nothing clever.
 */
@Singleton
class DataSyncSelectorSwarmImpl @Inject constructor(
    private val aapsLogger: AAPSLogger,
    private val preferences: Preferences,
    private val persistenceLayer: PersistenceLayer,
) : DataSyncSelector {

    /** Native dedupe state, held across the whole upload. */
    private var emitter: Long = 0L

    /** One queue to walk, expressed once instead of fourteen times. */
    private class Source(
        val name: String,
        val key: SwarmLongKey,
        val lastId: () -> Long?,
        val next: (Long) -> Pair<Any, Long>?,
        val valid: (Any) -> Boolean,
    )

    private val sources: List<Source> by lazy {
        listOf(
            Source("cgm", SwarmLongKey.GlucoseValueLastSyncedId,
                { persistenceLayer.getLastGlucoseValueId() },
                { id -> persistenceLayer.getNextSyncElementGlucoseValue(id).blockingGet()
                    ?.let { it.first to it.second.id } },
                { (it as app.aaps.core.data.model.GV).isValid }),
            Source("bolus", SwarmLongKey.BolusLastSyncedId,
                { persistenceLayer.getLastBolusId() },
                { id -> persistenceLayer.getNextSyncElementBolus(id).blockingGet()
                    ?.let { it.first to it.second.id } },
                { (it as app.aaps.core.data.model.BS).isValid }),
            Source("carb", SwarmLongKey.CarbsLastSyncedId,
                { persistenceLayer.getLastCarbsId() },
                { id -> persistenceLayer.getNextSyncElementCarbs(id).blockingGet()
                    ?.let { it.first to it.second.id } },
                { (it as app.aaps.core.data.model.CA).isValid }),
            Source("tbr", SwarmLongKey.TemporaryBasalLastSyncedId,
                { persistenceLayer.getLastTemporaryBasalId() },
                { id -> persistenceLayer.getNextSyncElementTemporaryBasal(id).blockingGet()
                    ?.let { it.first to it.second.id } },
                { (it as app.aaps.core.data.model.TB).isValid }),
            Source("extbolus", SwarmLongKey.ExtendedBolusLastSyncedId,
                { persistenceLayer.getLastExtendedBolusId() },
                { id -> persistenceLayer.getNextSyncElementExtendedBolus(id).blockingGet()
                    ?.let { it.first to it.second.id } },
                { (it as app.aaps.core.data.model.EB).isValid }),
            Source("event", SwarmLongKey.TherapyEventLastSyncedId,
                { persistenceLayer.getLastTherapyEventId() },
                { id -> persistenceLayer.getNextSyncElementTherapyEvent(id).blockingGet()
                    ?.let { it.first to it.second.id } },
                { (it as app.aaps.core.data.model.TE).isValid }),
            Source("target", SwarmLongKey.TemporaryTargetLastSyncedId,
                { persistenceLayer.getLastTemporaryTargetId() },
                { id -> persistenceLayer.getNextSyncElementTemporaryTarget(id).blockingGet()
                    ?.let { it.first to it.second.id } },
                { (it as app.aaps.core.data.model.TT).isValid }),
        )
    }

    override fun queueSize(): Long =
        sources.sumOf { source ->
            val last = source.lastId() ?: 0L
            (last - preferences.get(source.key)).coerceAtLeast(0L)
        }

    override fun resetToNextFullSync() {
        sources.forEach { preferences.remove(it.key) }
    }

    /**
     * The profile timestamp NSClient uses to avoid echoing a profile back.
     *
     * Irrelevant here: nothing is received, so nothing can be echoed. It is a
     * no-op rather than an error because a read-only plugin should be silent
     * about inbound events, not brittle about them.
     */
    override fun profileReceived(timestamp: Long) = Unit

    override suspend fun doUpload() {
        SwarmNative.check()
        emitter = SwarmNative.emitterNew()
        try {
            sources.forEach { drain(it) }
            val amendments = SwarmNative.emitterAmendments(emitter)
            if (amendments > 0) {
                // Recorded, not acted on. See spec §7 and the class comment.
                preferences.put(
                    SwarmLongKey.AmendmentsSeen,
                    preferences.get(SwarmLongKey.AmendmentsSeen) + amendments
                )
                aapsLogger.info(LTag.CORE, "swarm: $amendments post-emit edits this run")
            }
        } finally {
            SwarmNative.emitterFree(emitter)
            emitter = 0L
        }
    }

    private fun drain(source: Source) {
        while (true) {
            val lastDbId = source.lastId() ?: 0L
            var startId = preferences.get(source.key)
            if (startId > lastDbId) {
                // The database was restored or rolled back beneath us. Start
                // again rather than sitting past the end emitting nothing.
                preferences.put(source.key, 0L)
                startId = 0L
            }
            val (value, rowId) = source.next(startId) ?: break

            // The queue hands over retracted records; nothing upstream filters
            // them. See point 1 in the class comment.
            if (source.valid(value)) {
                SwarmRecords.from(value)?.let { raw ->
                    val line = SwarmNative.emitterAccept(emitter, raw)
                    if (line.isNotEmpty()) publish(line)
                }
            }

            // Advance past the row that triggered this, which may be a version
            // row rather than the record it resolved to.
            if (rowId > preferences.get(source.key)) preferences.put(source.key, rowId)
        }
    }

    /**
     * Where a canonical line goes.
     *
     * NOT IMPLEMENTED, AND NOT A STUB TO FILL IN CASUALLY. The next stage seals
     * these into epochs and wraps the epoch key to each live grantee — see
     * `tools/seal.py` for the reference and `spike/p2panda-seal` for what the
     * shipping key layer actually does. Until that is wired, this plugin
     * canonicalises and counts and publishes nothing, which is the correct
     * behaviour for a thing that has no swarm to publish to.
     */
    private fun publish(line: String) {
        aapsLogger.debug(LTag.CORE, "swarm: ${line.length} bytes canonicalised")
    }
}
