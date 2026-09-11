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
}
