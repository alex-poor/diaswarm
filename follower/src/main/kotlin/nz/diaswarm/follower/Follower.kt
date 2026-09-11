package nz.diaswarm.follower

import android.content.Context
import nz.diaswarm.jni.SwarmNative

/**
 * Everything this app knows, which is deliberately very little.
 *
 * **THERE IS NO DATABASE HERE, AND THAT IS THE WHOLE SHAPE OF THIS APP.** The
 * earlier follower was a flavour of AndroidAPS, so to draw a line it had to
 * write somebody else's glucose into AAPS's own `glucoseValues` table — which
 * meant a mirror, a high-water mark, tags to stop those rows being re-published
 * as your own, and an `EffectiveProfileSwitch` invented so the chart would draw
 * at all. None of that was about following anybody. It was the cost of
 * borrowing a loop app's screen.
 *
 * The vault IS the store. It already holds the records, sealed, on disk, and
 * `netGlucose` reads them. Asking it for the last few hours whenever the screen
 * needs redrawing is cheaper than maintaining a second copy, and there is no
 * second copy to disagree with the first.
 */
object Follower {

    /** The only purpose this app grants or reads under. */
    const val PURPOSE = "follow"

    /** One reading, as the vault gave it up. */
    data class Reading(val at: Long, val mgdl: Double, val trend: String)

    /**
     * One thing that was done, as the vault gave it up.
     *
     * **EVERY FIELD HERE WAS MEASURED AND SENT.** Nothing on this type is
     * computed, inferred or defaulted, which is what makes it safe to draw
     * beside somebody's glucose. [value] means units for a bolus, grams for
     * carbs, and a rate for a `tbr` whose unit is decided by [flag].
     */
    data class Treatment(
        val kind: String,
        val at: Long,
        val value: Double,
        val dur: Long,
        val flag: String
    ) {
        /** A `tbr` rate is U/h only when the emitter said so. */
        val absolute: Boolean get() = flag == "abs"
    }

    /** Somebody this phone follows. */
    data class Subject(val key: String, val purpose: String, val reached: Boolean) {
        /** Enough of the key to recognise, without pretending to be a name. */
        val short: String get() = key.take(8)
    }

    fun following(context: Context): List<Subject> {
        SwarmNative.check()
        return SwarmNative.netFollowing(SwarmPaths.store(context).absolutePath)
            .lines()
            .filter { it.isNotBlank() }
            .map { it.split('\t') }
            .map {
                Subject(
                    key = it.getOrElse(0) { "" },
                    purpose = it.getOrElse(1) { PURPOSE },
                    reached = it.getOrElse(2) { "0" } == "1"
                )
            }
            .filter { it.key.isNotEmpty() }
    }

    /**
     * Whose line the screen draws: the one they chose, or the only one there is.
     *
     * Following exactly one person is not an ambiguity, so it does not ask. With
     * several followed and none chosen, picking one would be picking a person at
     * random — so it shows the list instead.
     */
    fun chosen(context: Context): Subject? {
        val all = following(context).filter { it.reached }
        val named = Prefs.graphSubject(context)
        return when {
            named.isNotEmpty() -> all.firstOrNull { it.key.startsWith(named, ignoreCase = true) }
            all.size == 1      -> all.first()
            else               -> null
        }
    }

    /**
     * The readings for one subject over the last [hours].
     *
     * Read straight out of the vault on every refresh. `netGlucose` bounds the
     * decryption by epoch, so this opens the recent end of a grant rather than
     * the whole of somebody's history — the difference between a few segments
     * and a year of them.
     */
    fun readings(context: Context, subject: Subject, hours: Int): List<Reading> {
        SwarmNative.check()
        val since = System.currentTimeMillis() - hours * 3_600_000L
        return SwarmNative.netGlucose(
            SwarmPaths.store(context).absolutePath,
            subject.key,
            SwarmPaths.identity(context).absolutePath,
            subject.purpose,
            since,
            MAX_READINGS
        ).lines().filter { it.isNotBlank() }.mapNotNull { row ->
            val f = row.split('\t')
            val at = f.getOrNull(0)?.toLongOrNull() ?: return@mapNotNull null
            val mgdl = f.getOrNull(1)?.toDoubleOrNull() ?: return@mapNotNull null
            if (at <= 0 || mgdl <= 0.0) null else Reading(at, mgdl, f.getOrNull(2).orEmpty())
        }
    }

    /**
     * What was delivered to this subject over the last [hours].
     *
     * **THIS IS NOT NEW DATA, IT IS DATA THAT WAS BEING DISCARDED.** The
     * subject's phone has been sealing boluses, carbs and temporary basals
     * since the first drain; a granted follower has been decrypting them and
     * dropping them, because the only reader was `netGlucose` and it filters to
     * `cgm`. So this costs one more pass over segments that were opened anyway.
     *
     * **AND IT STILL CANNOT SHOW A PREDICTION.** Loop telemetry is excluded at
     * the emit boundary, so eventual-BG and the loop's own IOB are not late,
     * they were never published. Anything of that kind on this screen would
     * have to be computed here, and this app does not compute clinical numbers
     * on somebody else's behalf.
     */
    fun treatments(context: Context, subject: Subject, hours: Int): List<Treatment> {
        SwarmNative.check()
        val since = System.currentTimeMillis() - hours * 3_600_000L
        return SwarmNative.netTreatments(
            SwarmPaths.store(context).absolutePath,
            subject.key,
            SwarmPaths.identity(context).absolutePath,
            subject.purpose,
            since,
            MAX_TREATMENTS
        ).lines().filter { it.isNotBlank() }.mapNotNull { row ->
            val f = row.split('\t')
            val kind = f.getOrNull(0) ?: return@mapNotNull null
            val at = f.getOrNull(1)?.toLongOrNull() ?: return@mapNotNull null
            val value = f.getOrNull(2)?.toDoubleOrNull() ?: return@mapNotNull null
            if (at <= 0) null
            else Treatment(kind, at, value, f.getOrNull(3)?.toDouble()?.toLong() ?: 0L, f.getOrNull(4).orEmpty())
        }
    }

    /**
     * The target range this subject publishes, in mg/dL, or null.
     *
     * **THEIRS, NOT ONE THIS PHONE INVENTED.** A band is a clinical statement;
     * drawing somebody else's glucose against a stranger's targets would be
     * making one on their behalf. §2 requires profile records to carry target
     * blocks normalised to mg/dL precisely so a reader does not have to guess.
     */
    fun target(context: Context, subject: Subject): Pair<Double, Double>? {
        SwarmNative.check()
        val json = SwarmNative.netProfile(
            SwarmPaths.store(context).absolutePath,
            subject.key,
            SwarmPaths.identity(context).absolutePath,
            subject.purpose
        )
        if (json.isBlank()) return null
        return runCatching {
            val blocks = org.json.JSONObject(json).optJSONArray("target") ?: return null
            if (blocks.length() == 0) return null
            val first = blocks.getJSONObject(0)
            first.getDouble("lowTarget") to first.getDouble("highTarget")
        }.getOrNull()
    }

    /**
     * The scheduled basal rate, U/h, at [atMs] — the subject's own, published.
     *
     * **NEEDED BECAUSE A PERCENTAGE TEMP BASAL MEANS NOTHING WITHOUT IT.** A
     * `tbr` carries `rate` plus `abs`, and when `abs` is false the rate is a
     * percentage of whatever the profile was scheduling at that moment. The
     * subject publishes their basal blocks precisely so a consumer can resolve
     * that (spec §2: "without basal rates, ISF, IC and targets by time of day, a
     * consumer cannot say what the loop was trying to do, and the insulin
     * records become uninterpretable").
     *
     * So this is arithmetic on two published numbers, not a model of anybody's
     * insulin. Blocks are in order with their own durations in milliseconds,
     * covering the day from local midnight.
     */
    fun scheduledBasal(context: Context, subject: Subject, atMs: Long): Double {
        SwarmNative.check()
        val json = SwarmNative.netProfile(
            SwarmPaths.store(context).absolutePath,
            subject.key,
            SwarmPaths.identity(context).absolutePath,
            subject.purpose
        )
        if (json.isBlank()) return 0.0
        return runCatching {
            val blocks = org.json.JSONObject(json).optJSONArray("basal") ?: return 0.0
            val zone = java.util.TimeZone.getDefault()
            val cal = java.util.Calendar.getInstance(zone).apply { timeInMillis = atMs }
            val msIntoDay = ((cal.get(java.util.Calendar.HOUR_OF_DAY) * 3_600_000L) +
                (cal.get(java.util.Calendar.MINUTE) * 60_000L) +
                (cal.get(java.util.Calendar.SECOND) * 1_000L))
            var cursor = 0L
            for (i in 0 until blocks.length()) {
                val b = blocks.getJSONObject(i)
                val dur = b.optLong("duration", 0L)
                if (msIntoDay < cursor + dur || i == blocks.length() - 1) return b.optDouble("amount", 0.0)
                cursor += dur
            }
            0.0
        }.getOrDefault(0.0)
    }

    /** One step of effective basal delivery: [rate] U/h from [at] until the next. */
    data class BasalStep(val at: Long, val rate: Double)

    /**
     * The effective delivery rate over time, SAMPLED rather than taken record by
     * record.
     *
     * **TEMP BASALS OVERLAP AND SUPERSEDE EACH OTHER.** Each `tbr` carries the
     * duration it was requested for — half an hour, typically — but a loop that
     * re-decides every five minutes replaces it long before that expires.
     * Drawing one step per record stacks six of them at once and doubles back on
     * itself; it also implies rates that were never simultaneously in force.
     * The subject's own graph samples for exactly this reason, and this is a
     * port of that.
     *
     * A percentage rate is resolved against [scheduled] because that is what the
     * percentage is of; an absolute one is already U/h and is taken as it is.
     */
    fun basalSteps(
        treatments: List<Treatment>,
        scheduled: Double,
        from: Long,
        to: Long,
        sampleMs: Long = 300_000L
    ): List<BasalStep> {
        val tbrs = treatments.filter { it.kind == "tbr" }.sortedBy { it.at }
        if (to <= from) return emptyList()
        val out = ArrayList<BasalStep>()
        var t = from
        var last = Double.NaN
        while (t <= to) {
            val active = tbrs.lastOrNull { it.at <= t && (it.dur <= 0L || t < it.at + it.dur) }
            val rate = when {
                active == null -> scheduled
                active.absolute -> active.value
                else -> active.value / 100.0 * scheduled
            }
            if (last.isNaN() || kotlin.math.abs(rate - last) > 0.0001) {
                out.add(BasalStep(t, rate))
                last = rate
            }
            t += sampleMs
        }
        return out
    }

    /**
     * A reading's age is part of the reading.
     *
     * A follower's dangerous failure is not an error on screen: it is a number
     * that looks current and is nine hours old (§12.3). Nothing here returns a
     * glucose value without the means to say how old it is.
     */
    fun ageWords(at: Long, now: Long = System.currentTimeMillis()): String {
        val secs = (now - at) / 1000
        return when {
            secs < 90        -> "${secs.coerceAtLeast(0)}s ago"
            secs < 3600      -> "${secs / 60} min ago"
            secs < 7200      -> "1 hour ago"
            secs < 86_400    -> "${secs / 3600} hours ago"
            else             -> "${secs / 86_400} days ago"
        }
    }

    /** Bounds one read, and with it the string that crosses JNI. */
    private const val MAX_READINGS = 4_000

    /**
     * The same bound for treatments, and far smaller because they are far
     * rarer: this subject's busiest day in 74 is well under a hundred.
     */
    private const val MAX_TREATMENTS = 1_000
}
