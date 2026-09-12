package app.aaps.plugins.sync.swarm

import nz.diaswarm.jni.SwarmNative
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
         * How often the shadow vault is sealed. See [flushShadow].
         *
         * The CGM's own clinical cadence, and the bucket the loop reasons in.
         */
        private const val SHADOW_SEAL_INTERVAL_MS = 5 * 60 * 1000L

        /** Or sooner, if a backfill has handed over more than this at once. */
        private const val SHADOW_SEAL_BYTES = 32 * 1024

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
        const val STUCK_PASS_MS = 15 * 60 * 1000L

        /**
         * What gets published. Bump it and every shadow vault refills itself.
         *
         * 1 — CGM thinned to one reading per five minutes.
         * 2 — nothing thinned; spec §3.3 withdrawn.
         * 3 — the shadow is the `diaswarm-keys` vault, not the spaces one
         *     (D26), so an existing shadow directory holds a different vault's
         *     data and has to be refilled rather than added to.
         */
        const val SHADOW_GENERATION = 3L

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
                // NOTHING TO GUARD AGAINST ANY MORE. This used to reject rows
                // a follower had mirrored in from somebody else, so they were
                // not re-published as this subject's own glucose. The follower
                // is a separate app now and never writes here, so the only rows
                // in this table are ones this phone measured.
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
        // A STUCK PASS MUST NOT SILENCE THE PLUGIN FOR EVER.
        //
        // The flag is cleared in a `finally`, which covers a pass that throws
        // and not one that hangs — and one did: sealing opened a tokio runtime
        // per epoch, and a runtime that will not shut down blocks the pass
        // holding this flag. Every later pass then returned here immediately
        // and the plugin went quiet with nothing in the log to say why.
        //
        // So the flag has an age. A pass still "in flight" after fifteen
        // minutes is not in flight, it is lost, and the next one takes over.
        // The work is idempotent and the high-water marks make a repeat
        // harmless, so taking over is safer than waiting for ever.
        val startedAt = passStartedAt
        val stale = startedAt != 0L && System.currentTimeMillis() - startedAt > STUCK_PASS_MS
        if (stale) {
            aapsLogger.error(
                LTag.CORE,
                "swarm: previous pass has been running ${(System.currentTimeMillis() - startedAt) / 1000}s — taking over"
            )
            passInFlight.set(false)
        }
        if (!passInFlight.compareAndSet(false, true)) {
            aapsLogger.debug(LTag.CORE, "swarm: a pass is already running, skipping this one")
            return
        }
        passStartedAt = System.currentTimeMillis()
        try {
            uploadOnce()
        } finally {
            passStartedAt = 0L
            passInFlight.set(false)
        }
    }

    @Volatile
    private var passStartedAt = 0L

    private val passInFlight = java.util.concurrent.atomic.AtomicBoolean(false)

    private suspend fun uploadOnce() {
        SwarmNative.check()
        emitter = SwarmNative.emitterNew(preferences.get(SwarmLongKey.CgmBucketHighWater))
        try {
            backfillIfShadowJustEnabled()
            drained.clear()
            drainedThisPass = 0
            sources.forEach { drain(it) }
            if (drained.isNotEmpty()) {
                val by = drained.entries.sortedByDescending { it.value }
                    .joinToString(" ") { "${it.key}=${it.value}" }
                aapsLogger.info(LTag.CORE, "swarm: drained ${drained.values.sum()} — $by")
            }
            sealPending()
            // MORE TO DO, SO COME BACK. A bounded pass leaves the rest for the
            // next one, and without this nothing asks for a next one — a
            // re-drain would stop after 4,000 records and look finished.
            val more = drainedThisPass >= maxRecordsPerPass
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
            if (more) {
                aapsLogger.info(LTag.CORE, "swarm: pass full at $drainedThisPass, continuing")
                continuePass()
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

    /**
     * What the comparison found this pass, and why each is counted separately.
     *
     * `missing` is records handed to the shadow vault that did not come back
     * out of it — the number this feature exists to produce, and the one that
     * was never computed. `lost` is segments that would not open or lines that
     * would not parse. `failures` is the vault throwing or refusing, which used
     * to be the *only* thing shadow mode could detect.
     *
     * Zero of all three is the only passing result. They are summed over a pass
     * rather than collected, for the reason [shadowSealed] gives: the shadow
     * vault is worth nothing and must cost nothing.
     */
    private var shadowHeld = 0L
    private var shadowMissing = 0L
    private var shadowLost = 0L
    private var shadowFailures = 0L

    /**
     * Records waiting to be sealed into the spaces vault, held ACROSS passes.
     *
     * **BECAUSE AN OPERATION IS EXPENSIVE FOR EVER, NOT JUST ONCE.** The core
     * vault re-seals a whole segment, so sealing every pass costs it nothing
     * extra. The spaces vault appends an operation, and `opcost.rs` measured
     * what that means: per-operation read cost doubles as the count doubles,
     * and — the decisive part — delivering the same 2,000 operations in ten
     * chunks cost the same as delivering them at once, with each chunk dearer
     * than the last. The cost of an operation rises with how much history is
     * already held. Sealing one reading per pass was adding about 1,400
     * operations a day to a structure that charges more for each one.
     *
     * So the spaces path accumulates and seals on a cadence instead. The core
     * vault is untouched and still seals every pass; it is the authoritative
     * one and its freshness is what a follower reads.
     */
    private val shadowPending = mutableMapOf<Long, StringBuilder>()
    private var shadowSealedAt = 0L

    /**
     * The most a single pass will drain before stopping and coming back.
     *
     * **A BOUND, NOT A TUNING KNOB.** A full re-drain of 74 days is 31,000
     * records and about 2.7 MB of canonical text, and holding all of it —
     * `pending`, plus whatever AAPS allocates walking 80,000 rows — ran the app
     * out of its 268 MB heap and killed it. On this phone that stops the loop.
     *
     * Nothing about the drain needs to be atomic: the high-water marks advance
     * per row, so a pass that stops early resumes exactly where it left off.
     * Bounding it means no amount of history can make a pass too big, whatever
     * the phone is doing at the time.
     */
    private val maxRecordsPerPass = 4_000

    private var drainedThisPass = 0

    /**
     * Fill the shadow vault the first time shadow mode is switched on.
     *
     * A shadow vault holding only what arrived after somebody flipped a switch
     * cannot be compared against anything. Enabling the comparison should
     * therefore *cause* the comparison to be possible, rather than leaving a
     * person to find a second button and press that too.
     *
     * Resetting the marks is all it takes: the drain reads from wherever they
     * point, and the passes are bounded, so the history comes across in
     * four-thousand-record steps over the next few minutes.
     *
     * `CgmBucketHighWater` goes to -1, not 0. Zero is a real five-minute
     * bucket — just after midnight on 1 January 1970 — so zeroing it would
     * mean "everything since 1970 is already published" and thin every reading
     * in the backfill.
     */
    private fun backfillIfShadowJustEnabled() {
        if (!preferences.get(SwarmBooleanKey.ShadowSpacesVault)) return
        if (preferences.get(SwarmLongKey.ShadowFilled) == SHADOW_GENERATION) return

        // A GENERATION, NOT A FLAG, AND NOT A TRANSITION.
        //
        // The first version watched for the switch going off and then on, and
        // cleared its flag during a pass while off — so switching it off and
        // straight back on, which is what a person does, never cleared
        // anything and never triggered. A generation needs no timing at all:
        // it is bumped in the source whenever what gets published changes, and
        // any vault filled under an older one is refilled without being asked.
        //
        // Generation 2 is "nothing is thinned" (spec §3.3 withdrawn). A vault
        // filled at generation 1 is missing 14,620 CGM readings.
        aapsLogger.info(
            LTag.CORE,
            "swarm: shadow vault is generation " +
                "${preferences.get(SwarmLongKey.ShadowFilled)}, wanted $SHADOW_GENERATION " +
                "— re-reading everything"
        )
        for (key in SwarmLongKey.entries) {
            when (key) {
                SwarmLongKey.AmendmentsSeen, SwarmLongKey.ShadowFilled -> continue
                SwarmLongKey.CgmBucketHighWater -> preferences.put(key, -1L)
                else -> preferences.put(key, 0L)
            }
        }
        preferences.put(SwarmLongKey.ShadowFilled, SHADOW_GENERATION)
    }

    private fun drain(source: Source) {
        while (true) {
            if (drainedThisPass >= maxRecordsPerPass) return
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
                        drainedThisPass++
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
        val shadow = openShadow()
        // **NOTHING ACCUMULATES WHILE THE SHADOW IS OFF, AND IT USED TO.**
        //
        // `shadowPending` was appended to unconditionally, and `flushShadow`
        // returns at once when the handle is 0. Shadow mode is off by default,
        // so on every phone that has never enabled it — including the one
        // driving a pump — every record ever drained accumulated in a
        // StringBuilder in memory and was never written, read or freed. About
        // 160 KB a day at this subject's rate, for the lifetime of the process.
        //
        // `openShadow`'s own comment records an out-of-memory crash in this
        // same area. This is that shape again, and it was invisible because the
        // feature it belongs to is disabled.
        //
        // Found on a phone: the first verdict line read `given 17` where the
        // core vault had sealed 16, because one record left over from a pass
        // before the preference was switched on came along with the rest.
        // Harmless in itself — `missing 0` — and the only visible symptom of
        // something that is not harmless at all.
        if (shadow == 0L) shadowPending.clear()
        try {
        for ((epoch, body) in pending) {
            val text = body.toString()
            if (shadow != 0L) shadowPending.getOrPut(epoch) { StringBuilder() }.append(text)
            val n = SwarmNative.vaultSeal(vault, identity, epoch, offsetMs, text)
            if (n < 0) {
                aapsLogger.error(LTag.CORE, "swarm: sealing epoch $epoch failed with $n")
            } else {
                aapsLogger.info(LTag.CORE, "swarm: sealed epoch $epoch, $n records")
            }
        }
            flushShadow(shadow)
        } finally {
            // keysClose, NOT spacesClose. They take the same jlong and free
            // different types: closing a keys handle as a spaces one is a
            // type-confused Box::from_raw on a phone driving an insulin pump.
            // SwarmNative's own comment warns about exactly this — "two native
            // handle types reachable from Kotlin is a crash waiting for whoever
            // passes the wrong one" — and this line was that whoever.
            if (shadow != 0L) SwarmNative.keysClose(shadow)
        }
        pending.clear()
        aapsLogger.info(LTag.CORE, "swarm: ${SwarmNative.vaultStatus(vault)}")
        // ONE LINE THAT SAYS WHETHER IT AGREED, which is what four doc
        // comments and a user-facing preference summary have always claimed
        // this printed.
        if (shadowSealed > 0 || shadowFailures > 0) {
            val agreed = shadowMissing == 0L && shadowLost == 0L && shadowFailures == 0L
            val line =
                "swarm: shadow ${if (agreed) "agrees" else "DISAGREES"} — " +
                    "given $shadowSealed, holds $shadowHeld, missing $shadowMissing, " +
                    "lost $shadowLost, failures $shadowFailures"
            // AT THE LEVEL THE VERDICT DESERVES. A disagreement logged at info
            // is a disagreement nobody greps for.
            if (agreed) aapsLogger.info(LTag.CORE, line) else aapsLogger.error(LTag.CORE, line)
            shadowSealed = 0L
            shadowHeld = 0L
            shadowMissing = 0L
            shadowLost = 0L
            shadowFailures = 0L
        }
    }

    /**
     * Open the shadow vault once, for the whole pass.
     *
     * **ONCE, NOT PER EPOCH, AND THE DIFFERENCE WEDGED THE PLUGIN.** Opening it
     * inside the epoch loop — which is how the fix for the out-of-memory crash
     * left it — builds a fresh tokio runtime, SQLite connection and p2panda
     * manager for every day, and tears each one down again. A full drain is 74
     * of those. `Runtime::drop` blocks until its tasks finish, so one that does
     * not terminate promptly hangs `sealPending`, which hangs the pass, which
     * leaves `passInFlight` set for ever and makes every later pass return
     * immediately. The symptom is a plugin that has silently stopped.
     */
    private fun openShadow(): Long {
        if (!preferences.get(SwarmBooleanKey.ShadowSpacesVault)) return 0L
        return try {
            // **THE KEYS VAULT, NOT THE SPACES ONE.** D26 decided against the
            // spaces message layer — it cannot express a follower who reads
            // only the last day — so shadowing it was measuring the thing that
            // is not going to ship. Its JNI is still there and still measured;
            // nothing calls it from here.
            // **THE POOL OWNS THE STORE**, so this borrows rather than opening
            // a second connection to the same file — see SwarmPlugin's note.
            // No pool means no keys store, and shadow mode simply does nothing.
            val pool = SwarmEndpoint.handle
            if (pool == 0L) return 0L
            val dir = SwarmKeys.dir(context).absolutePath
            val identity = SwarmPaths.identity(context).absolutePath
            SwarmNative.keysOpen(pool, dir, identity, offsetMs).also {
                if (it == 0L) aapsLogger.error(LTag.CORE, "swarm: shadow vault would not open")
            }
        } catch (e: Throwable) {
            aapsLogger.error(LTag.CORE, "swarm: shadow vault threw on open: $e")
            0L
        }
    }

    /**
     * Seal one day into the shadow vault too, and say whether it agrees.
     *
     * **NOTHING DEPENDS ON THE RESULT.** The real vault stays authoritative;
     * this writes to a separate directory, is read by no screen, and its worst
     * failure is a log line.
     *
     * Off by default. See [SwarmBooleanKey.ShadowSpacesVault].
     */
    /**
     * Seal the accumulated records into the spaces vault, on a cadence.
     *
     * **FIVE MINUTES, AND THE NUMBER IS A TRADE RATHER THAN A TUNING.** It was
     * chosen when the shadow was the spaces vault, whose cost is quadratic in
     * the operation count: one seal a pass is about 1,400 a day, one every five
     * minutes is 288, and the read cost fell by roughly 25×.
     *
     * **THE KEYS VAULT DOES NOT HAVE THAT COST, AND THE CADENCE STILL EARNS ITS
     * PLACE.** One segment per epoch means sealing is flat in the number of
     * flushes — but each flush now re-opens the accumulated day, merges, and
     * re-seals it, so flushing on every pass would re-encrypt a growing day
     * some 1,400 times instead of 288. Same direction, smaller stakes.
     *
     * Five is where it stops being free. It is the cadence the loop itself
     * reasons in, it is what `spec/records.md` buckets to, and the follower
     * polls every two minutes against a sensor that reports every one — so a
     * reading waits at most one CGM cycle longer than it already did. An hour
     * would be twelve times cheaper again and would make a follower useless at
     * 3 a.m., which is the use case this project exists for.
     *
     * **A CLOSED DAY IS NEVER HELD.** An epoch that is no longer the current one
     * is sealed immediately whatever the clock says, so a day cannot sit in
     * memory waiting for a cadence that only fires while records are arriving.
     *
     * **AND IT IS SAFE TO LOSE.** If the process dies with records accumulated,
     * they are missing from the shadow vault until a re-drain. That vault is
     * authoritative for nothing, is read by no screen, and D21's shadow mode
     * exists precisely so its failures cost a log line. The core vault still
     * seals every pass and is what a follower reads.
     */
    private fun flushShadow(handle: Long) {
        if (handle == 0L || shadowPending.isEmpty()) return
        val now = System.currentTimeMillis()
        val currentEpoch = SwarmNative.epochOf(now, offsetMs)
        val due = shadowSealedAt == 0L || now - shadowSealedAt >= SHADOW_SEAL_INTERVAL_MS
        val bulky = shadowPending.values.sumOf { it.length } >= SHADOW_SEAL_BYTES

        // Closed days go now; today waits for the cadence.
        val ready = shadowPending.keys.filter { it != currentEpoch || due || bulky }
        if (ready.isEmpty()) return

        for (epoch in ready) {
            shadowPending.remove(epoch)?.let { shadowSeal(handle, epoch, it.toString()) }
        }
        shadowSealedAt = now
    }

    /**
     * Seal one epoch into the shadow vault, and check it came back.
     *
     * **THIS IS WHERE SHADOW MODE STOPPED BEING A CLAIM.** Four places said it
     * logs "whether they agree", including the preference summary a user reads.
     * What the code did was add up how many records the shadow vault reported
     * sealing and print that number beside an unrelated status line from the
     * other vault. Nothing was compared with anything. A shadow vault that
     * silently kept four days out of five read exactly like one working
     * perfectly — which is the failure shape this project keeps being bitten
     * by, sitting inside the component whose job is catching it.
     *
     * **AND "AGREE" NOW MEANS SOMETHING STRONGER.** Not "the two vaults agree
     * with each other" — the old vault is the thing being replaced and is
     * itself fallible, and two vaults can agree by losing the same record — but
     * "the new vault gives back exactly the records it was handed".
     * `keysSealChecked` seals, re-opens the segment from disk, and counts what
     * did not come back.
     */
    private fun shadowSeal(handle: Long, epoch: Long, ndjson: String) {
        if (handle == 0L || ndjson.isBlank()) return
        try {
            val report = SwarmNative.keysSealChecked(handle, epoch, ndjson)
            val fields = report.split(' ')
                .mapNotNull { f -> f.split('=', limit = 2).takeIf { it.size == 2 } }
                .associate { it[0] to it[1] }

            if (!report.startsWith("ok ")) {
                shadowFailures += 1
                aapsLogger.error(LTag.CORE, "swarm: shadow seal: $report")
                return
            }
            // **A REPORT THIS CANNOT READ IS A FAILURE, NOT A ZERO.** Falling
            // back to 0 for a field that is not there turns "the Kotlin and the
            // Rust disagree about the format" into "missing 0", which reads as
            // agreement — the same silent-success shape this whole change
            // exists to remove. The contract is one line of `k=v` pairs from
            // `keysSealChecked`; if it is not that, say so.
            val given = fields["given"]?.toLongOrNull()
            val held = fields["held"]?.toLongOrNull()
            val missing = fields["missing"]?.toLongOrNull()
            val lost = fields["lost"]?.toLongOrNull()
            if (given == null || held == null || missing == null || lost == null) {
                shadowFailures += 1
                aapsLogger.error(LTag.CORE, "swarm: shadow report not understood: $report")
                return
            }
            shadowSealed += given
            shadowHeld += held
            shadowMissing += missing
            shadowLost += lost

            // A disagreement is logged at the moment it happens, with the epoch,
            // rather than only as a total at the end of the pass. A total tells
            // you something is wrong; the epoch tells you which day to look at.
            if (missing > 0 || lost > 0) {
                aapsLogger.error(LTag.CORE, "swarm: shadow DISAGREES: $report")
            }
        } catch (e: Throwable) {
            // CAUGHT, INCLUDING ERRORS. This runs on a phone driving an insulin
            // pump and is worth precisely nothing; an UnsatisfiedLinkError from
            // a stale .so must not take the sync worker down with it.
            shadowFailures += 1
            aapsLogger.error(LTag.CORE, "swarm: shadow seal threw: $e")
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
        // **CARRY THE KEYS LOGS TOO.** Without this the replicator exists,
        // subscribes to nothing, and every keys read finds an empty store —
        // which looks exactly like "not granted yet".
        val carried = SwarmNative.keysCarryAll(
            handle,
            SwarmPaths.store(context).absolutePath,
            SwarmPaths.identity(context).absolutePath
        )
        // **SAY WHAT IT DID, NOT ONLY WHEN IT FAILED.** The first version of
        // this logged nothing on success, so a pass that carried nothing and a
        // pass that carried everything read identically — which is the exact
        // failure shape the rest of this file exists to avoid, written by
        // somebody who had spent the day removing it elsewhere.
        when {
            carried < 0 -> aapsLogger.info(LTag.CORE, "swarm: keys carry unavailable ($carried)")
            carried == 0L -> aapsLogger.info(LTag.CORE, "swarm: keys carrying nothing")
            else -> aapsLogger.info(LTag.CORE, "swarm: keys carrying $carried log(s)")
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
    /**
     * Ask for another pass, shortly, because this one filled up.
     *
     * **UNDER ITS OWN NAME.** The first version of this enqueued with `REPLACE`
     * against `SwarmPlugin.JOB_NAME` — the unique name of the work it is
     * running *inside*. So a pass cancelled itself to schedule its own
     * replacement, which cancelled itself in turn: no drain ever finished, and
     * the only trace was `JobScheduler: Job didn't exist in JobStore` three
     * times a minute. A continuation must never be able to cancel the pass
     * that asked for it.
     *
     * A short delay rather than none: the pass queueing this is still holding
     * its own records, and stacking the next one immediately puts two passes'
     * worth of history in the heap at the same time — which is what the bound
     * exists to prevent.
     */
    private fun continuePass() {
        WorkManager.getInstance(context).beginUniqueWork(
            SwarmPlugin.CONTINUE_JOB_NAME,
            ExistingWorkPolicy.REPLACE,
            OneTimeWorkRequest.Builder(SwarmDataSyncWorker::class.java)
                .setInitialDelay(5, TimeUnit.SECONDS)
                .build()
        ).enqueue()
    }

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

        preferences.get(SwarmStringKey.GrantReader).trim().takeIf { it.isNotEmpty() }?.let { asked ->
            // Either a bare 64-hex reader key, as this field has always taken,
            // or a whole invite scanned from their screen. An invite carries
            // where they are as well as who they are, which is what lets this
            // finish the job without them scanning anything back.
            val fields = SwarmNative.inviteParse(asked).split('\t')
            val who = fields.getOrNull(0)?.takeIf { it.isNotEmpty() } ?: asked

            val n = SwarmNative.vaultGrant(vault, identity, who, PURPOSE)
            preferences.put(SwarmStringKey.GrantReader, "")
            if (n < 0) {
                aapsLogger.error(LTag.CORE, "swarm: grant refused ($n) for ${who.take(16)}…")
                return@let
            }
            aapsLogger.info(LTag.CORE, "swarm: granted ${who.take(16)}… — $n wraps published")

            // **AND ON THE KEYS VAULT TOO, IF THEY PUBLISHED AN IDENTITY.**
            // Without this a scan grants half of what the invite offers: the
            // reader gets wraps for the old vault and is not a member of the
            // new one, so the day the old vault goes away they stop reading
            // with nothing to say why. Empty is a v1 or v2 invite — somebody
            // whose app has no keys vault — and is not a failure.
            val theirKeys = fields.getOrNull(4).orEmpty()
            if (theirKeys.isNotEmpty()) {
                val handle = openShadow()
                if (handle == 0L) {
                    aapsLogger.error(LTag.CORE, "swarm: keys grant skipped — no keys vault")
                } else {
                    try {
                        val tag = SwarmNative.keysGrant(handle, theirKeys, PURPOSE)
                        if (tag.startsWith("error")) {
                            aapsLogger.error(LTag.CORE, "swarm: keys grant failed: $tag")
                        } else {
                            aapsLogger.info(LTag.CORE, "swarm: keys granted as ${tag.take(16)}…")
                        }
                    } catch (e: Throwable) {
                        aapsLogger.error(LTag.CORE, "swarm: keys grant threw: $e")
                    } finally {
                        SwarmNative.keysClose(handle)
                    }
                }
            }

            // AND TELL THEM WHERE TO LOOK. Granting somebody who cannot find
            // you is half a share; they would otherwise have to scan a second
            // code to learn an address we already know.
            val theirEndpoint = fields.getOrNull(1).orEmpty()
            if (theirEndpoint.isEmpty()) return@let
            val ours = ownInvite()
            if (ours.isEmpty()) {
                aapsLogger.info(LTag.CORE, "swarm: granted, but this phone has no invite to offer yet")
                return@let
            }
            when (val sent = SwarmNative.swarmOffer(
                SwarmEndpoint.handle, theirEndpoint, fields.getOrNull(3).orEmpty(), ours
            )) {
                1L   -> aapsLogger.info(LTag.CORE, "swarm: handed them our invite — they are following now")
                0L   -> aapsLogger.info(
                    LTag.CORE,
                    "swarm: they were not expecting an invite — they can scan ours instead"
                )
                else -> aapsLogger.info(LTag.CORE, "swarm: could not reach them to hand our invite over ($sent)")
            }
        }

        // FOLLOWING IS NOT GRANTING, and it does not touch this phone's vault —
        // it is here because this is the one place that acts on what the user
        // asked for and clears the request, and a second mechanism for that
        // would be a second thing to get wrong.
        preferences.get(SwarmStringKey.FollowInvite).trim().takeIf { it.isNotEmpty() }?.let { invite ->
            val store = SwarmPaths.store(context).absolutePath
            val n = SwarmNative.netFollow(store, invite)
            preferences.put(SwarmStringKey.FollowInvite, "")
            when {
                n > 0L  -> aapsLogger.info(LTag.CORE, "swarm: now following ${invite.take(16)}…")
                n == 0L -> aapsLogger.info(LTag.CORE, "swarm: already following ${invite.take(16)}…")
                else    -> aapsLogger.error(LTag.CORE, "swarm: that invite would not parse ($n)")
            }
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
     * This phone's own invite, or empty when it has nothing to offer yet.
     *
     * Needs both halves: a subject key, which exists once anything is sealed,
     * and an endpoint id, which exists only while serving.
     */
    private fun ownInvite(): String {
        val handle = SwarmEndpoint.handle
        if (handle == 0L) return ""
        val subject = SwarmNative.vaultSubject(SwarmPaths.identity(context).absolutePath)
        val endpoint = SwarmNative.swarmNodeId(handle)
        if (subject.isEmpty() || endpoint.isEmpty()) return ""
        return SwarmNative.inviteFor(subject, endpoint, PURPOSE, SwarmKeys.identity(context, preferences))
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
