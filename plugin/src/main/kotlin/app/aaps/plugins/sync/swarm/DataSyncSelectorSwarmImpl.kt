package app.aaps.plugins.sync.swarm

import androidx.work.ExistingWorkPolicy
import androidx.work.OneTimeWorkRequest
import androidx.work.WorkManager
import app.aaps.plugins.sync.swarm.workers.SwarmDataSyncWorker
import java.util.concurrent.TimeUnit
import android.content.Context
import app.aaps.core.interfaces.db.PersistenceLayer
import app.aaps.core.interfaces.logging.AAPSLogger
import app.aaps.core.interfaces.logging.LTag
import app.aaps.core.interfaces.sync.DataSyncSelector
import app.aaps.core.keys.interfaces.Preferences
import app.aaps.plugins.sync.swarm.keys.SwarmBooleanKey
import app.aaps.plugins.sync.swarm.keys.SwarmLongKey
import app.aaps.plugins.sync.swarm.keys.SwarmStringKey
import org.json.JSONObject
import java.io.File
import java.util.TimeZone
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
    private val context: Context,
) : DataSyncSelector {

    /** Native dedupe state, held across the whole upload. */
    private var emitter: Long = 0L

    companion object {

        /**
         * The only purpose the phone grants under, for now.
         *
         * `clinician` and `cohort` are different key trees (§7.2) and want a UI
         * that says which one you are handing over — offering them through a
         * preference nobody can see would be worse than not offering them.
         */
        const val PURPOSE = "follow"

        /**
         * How often a following phone asks again.
         *
         * A CGM produces a reading every five minutes, so polling faster than
         * that mostly discovers nothing has changed — which costs one QUIC
         * connection and a manifest, because an unchanged subject transfers no
         * segments and no wraps. Two minutes keeps the worst case comfortably
         * under the data's own cadence without polling into the gaps.
         */
        const val FOLLOW_POLL_SECONDS = 120L
    }

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
            Source("profile", SwarmLongKey.ProfileSwitchLastSyncedId,
                { persistenceLayer.getLastProfileSwitchId() },
                { id -> persistenceLayer.getNextSyncElementProfileSwitch(id).blockingGet()
                    ?.let { it.first to it.second.id } },
                { (it as app.aaps.core.data.model.PS).isValid }),
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
        // ONE PASS AT A TIME.
        //
        // The one-shot and periodic jobs are separate unique works, so
        // WorkManager will happily run them together — observed as two workers
        // logging an identical drain at the same millisecond. Two passes share
        // `emitter`, `pending` and `drained`: one frees the native emitter in
        // its `finally` while the other is still calling into it, which is a
        // use-after-free in JNI on a phone driving a pump.
        //
        // Skipping rather than queueing, because the work is idempotent: the
        // high-water marks mean the next pass picks up whatever this one would
        // have done.
        if (!passInFlight.compareAndSet(false, true)) {
            aapsLogger.debug(LTag.CORE, "swarm: a pass is already running, skipping this one")
            return
        }
        try {
            uploadOnce()
        } finally {
            passInFlight.set(false)
        }
    }

    private val passInFlight = java.util.concurrent.atomic.AtomicBoolean(false)

    private suspend fun uploadOnce() {
        SwarmNative.check()
        emitter = SwarmNative.emitterNew(preferences.get(SwarmLongKey.CgmBucketHighWater))
        try {
            drained.clear()
            sources.forEach { drain(it) }
            if (drained.isNotEmpty()) {
                val by = drained.entries.sortedByDescending { it.value }
                    .joinToString(" ") { "${it.key}=${it.value}" }
                aapsLogger.info(LTag.CORE, "swarm: drained ${drained.values.sum()} — $by")
            }
            sealPending()
            applyPendingGrants()
            refreshFollowed()
            poolPass()
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
            run {
                // BEFORE FREEING IT. The mark lives in the emitter and dies
                // with it; without this, thinning restarts from nothing every
                // pass and a one-minute sensor publishes every reading.
                val mark = SwarmNative.emitterLastCgmBucket(emitter)
                if (mark >= 0) preferences.put(SwarmLongKey.CgmBucketHighWater, mark)
                val thinned = SwarmNative.emitterThinned(emitter)
                if (thinned > 0) {
                    aapsLogger.info(LTag.CORE, "swarm: thinned $thinned CGM readings to 5-minute buckets")
                }
            }
            SwarmNative.emitterFree(emitter)
            emitter = 0L
        }
    }

    /**
     * How many records each source contributed this pass.
     *
     * **A TOTAL CANNOT BE RECONCILED WITH ANOTHER TOTAL.** The device emitted
     * 35,897 records where `tools/canon.py` produced 31,341 from the same
     * database; 3,898 of the gap was CGM thinning and 1 was a post-emit edit,
     * and 563 could not be attributed to anything because neither side could
     * say which *kind* of record it disagreed about. Counting per kind is what
     * turns "563 somewhere" into a table.
     */
    private val drained = mutableMapOf<String, Int>()

    /**
     * Shadow records sealed this pass, summed rather than collected.
     *
     * **A COUNT, NOT A LIST.** The first version built a second copy of every
     * canonical line in the pass and then joined it into one string to hand
     * across JNI. On a full re-drain — 31,000 records, 2.7 MB of text — that
     * was a duplicate of the whole history plus a single allocation as large
     * again, on top of `pending` and whatever AAPS needs to walk 80,000 rows.
     * It ran the app out of heap and killed it, which on this phone means the
     * loop stops. The shadow vault is worth nothing and must cost nothing.
     */
    private var shadowSealed = 0L

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
                    if (line.isNotEmpty()) {
                        publish(line)
                        drained[source.name] = (drained[source.name] ?: 0) + 1
                    }
                }
            }

            // Advance past the row that triggered this, which may be a version
            // row rather than the record it resolved to.
            if (rowId > preferences.get(source.key)) preferences.put(source.key, rowId)
        }
    }

    /**
     * Canonical lines, grouped by the UTC day they belong to.
     *
     * Held until the drain finishes rather than sealed per record, because
     * sealing rewrites a whole epoch: doing it once per record would rewrite
     * the day hundreds of times for one pass.
     */
    private val pending = mutableMapOf<Long, StringBuilder>()

    private fun publish(line: String) {
        // PARSED, NOT SCANNED. Finding `"t":` by string search reads the first
        // occurrence, and canonical keys are sorted — so `note`, which §2 calls
        // free text a person typed, comes before `t`. A note containing `"t":`
        // would silently file the record under the wrong day, or none.
        val epoch = try {
            SwarmNative.epochOf(JSONObject(line).getLong("t"), offsetMs)
        } catch (e: Exception) {
            aapsLogger.error(LTag.CORE, "swarm: unparseable canonical line, dropped")
            return
        }
        pending.getOrPut(epoch) { StringBuilder() }.append(line).append('\n')
    }

    /**
     * Seal each day this pass collected.
     *
     * A day is sealed again every time more of it arrives, and that is safe:
     * the epoch keeps its key across re-seals, so wraps already published for
     * it keep opening. Minting a fresh key per seal would leave readers holding
     * a key that opens nothing, with no error anywhere to say so.
     *
     * TODAY IS SEALED TOO, not held back until it closes. A follower wants the
     * day as it happens, and there is no reason to withhold it once re-sealing
     * is safe — but note nothing publishes these bytes anywhere yet.
     */
    private fun sealPending() {
        val vault = SwarmPaths.vault(context, this::class.java).absolutePath
        val identity = SwarmPaths.identity(context).absolutePath
        for ((epoch, body) in pending) {
            val text = body.toString()
            shadowSeal(text)
            val n = SwarmNative.vaultSeal(vault, identity, epoch, offsetMs, text)
            if (n < 0) {
                aapsLogger.error(LTag.CORE, "swarm: sealing epoch $epoch failed with $n")
            } else {
                aapsLogger.info(LTag.CORE, "swarm: sealed epoch $epoch, $n records")
            }
        }
        pending.clear()
        aapsLogger.info(LTag.CORE, "swarm: ${SwarmNative.vaultStatus(vault)}")
        if (shadowSealed > 0) {
            aapsLogger.info(LTag.CORE, "swarm: shadow sealed $shadowSealed")
            shadowSealed = 0L
        }
    }

    /**
     * Seal the same records into the spaces vault too, and say whether it agrees.
     *
     * **NOTHING DEPENDS ON THE RESULT.** The vault above stays authoritative;
     * this writes to a separate directory, is read by no screen, and its worst
     * failure is a log line. That is the whole point: D20 and D21 agree with
     * the old vault over 74 days of real history and run on this hardware, and
     * neither of those is the same as having run inside AAPS for a week.
     *
     * Off by default. See [SwarmBooleanKey.ShadowSpacesVault].
     */
    private fun shadowSeal(ndjson: String) {
        if (!preferences.get(SwarmBooleanKey.ShadowSpacesVault)) return
        if (ndjson.isBlank()) return

        var handle = 0L
        try {
            val dir = File(SwarmPaths.base(context), "spaces").absolutePath
            handle = SwarmNative.spacesOpen(dir, offsetMs)
            if (handle == 0L) {
                aapsLogger.error(LTag.CORE, "swarm: shadow vault would not open")
                return
            }
            val n = SwarmNative.spacesSeal(handle, ndjson)
            if (n < 0) {
                aapsLogger.error(LTag.CORE, "swarm: shadow seal failed with $n")
            } else {
                shadowSealed += n
            }
        } catch (e: Throwable) {
            // CAUGHT, INCLUDING ERRORS. This runs on a phone driving an insulin
            // pump and is worth precisely nothing; an UnsatisfiedLinkError from
            // a stale .so must not take the sync worker down with it.
            aapsLogger.error(LTag.CORE, "swarm: shadow seal threw: $e")
        } finally {
            if (handle != 0L) SwarmNative.spacesClose(handle)
        }
    }

    /**
     * Take part in the pool: say what we hold, hear what we should, take some.
     *
     * Joining is not participating, and the difference is easy to miss. A phone
     * that joins and never ticks appears in the pool, is counted on by every
     * peer computing its share, and holds nothing for anybody — which is worse
     * than not being there, because the peers that would otherwise have covered
     * that slice believe it is covered.
     *
     * Two subjects a pass. This runs inside the sync worker on a phone driving
     * an insulin pump, and fetching somebody's year of history is not something
     * to do all at once on mobile data.
     */
    private fun poolPass() {
        val handle = SwarmEndpoint.handle
        if (handle == 0L) return
        val report = SwarmNative.swarmTick(handle, 2)
        if (report.isEmpty()) {
            aapsLogger.debug(LTag.CORE, "swarm: pool pass failed")
            return
        }
        val f = report.split('\t')
        aapsLogger.info(
            LTag.CORE,
            "swarm: pool ${f.getOrElse(0) { "?" }} peers, ${f.getOrElse(1) { "?" }} buckets, " +
                "holding ${f.getOrElse(2) { "?" }}, want ${f.getOrElse(3) { "?" }}, " +
                "took ${f.getOrElse(4) { "?" }}"
        )
    }

    /**
     * Pull anything new from the people this phone follows.
     *
     * On the same pass as the outbound drain, because it is the same question
     * — "has anything changed?" — and a follower that only refreshed when the
     * user opened a screen would show whatever was true last time they looked.
     * §12.3 is about exactly this: a reading that is stale and does not say so.
     *
     * Costs nothing when nothing has changed: the manifest carries sizes and a
     * wrap count, so an unchanged subject transfers no bytes at all. Failures
     * are logged and dropped — an unreachable peer is the ordinary condition of
     * a swarm, not an error, and the freshness display is what tells the user
     * it has been going on too long.
     */
    private fun refreshFollowed() {
        val store = SwarmPaths.store(context).absolutePath
        val follows = SwarmNative.netFollowing(store).lines().count { it.isNotBlank() }
        if (follows == 0) return

        val reached = SwarmNative.netRefresh(SwarmEndpoint.handle, store)
        if (reached < 0) aapsLogger.debug(LTag.CORE, "swarm: refresh failed ($reached)")
        else if (reached > 0) aapsLogger.debug(LTag.CORE, "swarm: refreshed $reached followed subject(s)")

        scheduleNextPoll()
    }

    /**
     * Come back in two minutes, but only on a phone that follows somebody.
     *
     * WHY NOT JUST THE PERIODIC JOB. Fifteen minutes is WorkManager's floor for
     * periodic work, and for someone watching glucose it is useless: a reading
     * arrives every five minutes and they would see it a quarter of an hour
     * later, which is worse than the thing this is meant to replace. A one-time
     * job can carry any delay it likes and re-arm itself, so this does that; the
     * periodic job stays underneath as the thing that restarts the chain after
     * the process is killed.
     *
     * WHY IT IS CONDITIONAL. The same code runs on a phone driving an insulin
     * pump, and waking that phone every two minutes to ask a question it has no
     * reason to ask is a battery cost for nothing. A publisher follows nobody,
     * so it never gets here.
     *
     * IT IS NOT A GUARANTEE. Doze batches this like everything else, so "two
     * minutes" means two minutes while the phone is awake and something longer
     * while it is in a pocket overnight. That is the platform, not the design —
     * and it is why every reading is shown with its age rather than as a number
     * that implies it is current.
     */
    private fun scheduleNextPoll() {
        WorkManager.getInstance(context).beginUniqueWork(
            SwarmPlugin.FOLLOW_JOB_NAME,
            ExistingWorkPolicy.REPLACE,
            OneTimeWorkRequest.Builder(SwarmDataSyncWorker::class.java)
                .setInitialDelay(FOLLOW_POLL_SECONDS, TimeUnit.SECONDS)
                .build()
        ).enqueue()
    }

    /**
     * Act on a grant or withdrawal the user asked for, then clear the request.
     *
     * Cleared whether it succeeded or not: a request left in place would be
     * retried on every pass forever, and a malformed key would retry forever
     * silently. The log line is the record of what happened.
     */
    private fun applyPendingGrants() {
        val vault = SwarmPaths.vault(context, this::class.java).absolutePath
        val identity = SwarmPaths.identity(context).absolutePath

        preferences.get(SwarmStringKey.GrantReader).trim().takeIf { it.isNotEmpty() }?.let { who ->
            val n = SwarmNative.vaultGrant(vault, identity, who, PURPOSE)
            preferences.put(SwarmStringKey.GrantReader, "")
            if (n < 0) aapsLogger.error(LTag.CORE, "swarm: grant refused ($n) for ${who.take(16)}…")
            else aapsLogger.info(LTag.CORE, "swarm: granted ${who.take(16)}… — $n wraps published")
        }

        preferences.get(SwarmStringKey.RevokeReader).trim().takeIf { it.isNotEmpty() }?.let { who ->
            val from = SwarmNative.vaultRevoke(vault, identity, who, PURPOSE)
            preferences.put(SwarmStringKey.RevokeReader, "")
            if (from < 0) aapsLogger.error(LTag.CORE, "swarm: withdrawal refused ($from)")
            else aapsLogger.info(
                LTag.CORE,
                "swarm: withdrew ${who.take(16)}… from segment $from — immediate"
            )
        }
    }

    /**
     * Where the vault lives.
     *
     * App-private storage. It holds ciphertext and the subject's own key
     * material, and it must never be written anywhere another app can read —
     * the epoch keys beside the sealed data are what make every later grant
     * possible, and they are the one thing here that is not safe to leak.
     */
    private val storage: File by lazy {
        File(context.filesDir, "diaswarm").also { it.mkdirs() }
    }

    /**
     * The phase epochs are cut at: this device's standing offset from UTC.
     *
     * `rawOffset`, not the current offset — it excludes daylight saving, which
     * is what "fixed" means here. An epoch that shifts twice a year is an epoch
     * two implementations can disagree about, and a record's epoch must not
     * depend on the time of year it was written.
     *
     * It is read once and then recorded in the vault, so moving zone later does
     * not silently re-cut a history that was already sealed at another phase.
     */
    private val offsetMs: Long by lazy { TimeZone.getDefault().rawOffset.toLong() }
}
