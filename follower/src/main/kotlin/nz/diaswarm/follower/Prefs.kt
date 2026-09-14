package nz.diaswarm.follower

import android.content.Context

/**
 * The four things this app remembers.
 *
 * `SharedPreferences` and nothing more. The AAPS add-on needs a typed key
 * registry because unregistered keys are invisible to settings export and
 * render as dead rows; here there is no export and no generated screen, so a
 * registry would be ceremony around four strings.
 */
object Prefs {

    private const val FILE = "diaswarm-follower"
    private const val GRAPH_SUBJECT = "graph_subject"
    private const val RANGE_HOURS = "range_hours"
    private const val UNITS_MMOL = "units_mmol"
    private const val LOW_LINE = "low_line_mgdl"
    private const val HIGH_LINE = "high_line_mgdl"
    private const val KEYS_VAULT = "keys_vault"
    private const val HANDED_OVER_AT = "handed_over_at_"
    private const val KEYS_ONLY = "keys_only"
    private const val STAY_REACHABLE = "stay_reachable"
    private const val LAST_RESTART = "last_endpoint_restart"

    private fun p(c: Context) = c.getSharedPreferences(FILE, Context.MODE_PRIVATE)

    /** Whose line to draw, by the start of their key. Blank means "the only one". */
    fun graphSubject(c: Context): String = p(c).getString(GRAPH_SUBJECT, "").orEmpty().trim()
    fun setGraphSubject(c: Context, v: String) = p(c).edit().putString(GRAPH_SUBJECT, v.trim()).apply()

    /** Hours of history on screen. */
    fun rangeHours(c: Context): Int = p(c).getInt(RANGE_HOURS, 6)
    fun setRangeHours(c: Context, v: Int) = p(c).edit().putInt(RANGE_HOURS, v).apply()

    /**
     * Display units.
     *
     * mmol/L by default, because this was built in a country that uses it and a
     * wrong default is one tap to fix. The stored records are always mg/dL —
     * §2 normalises on the way out — so this is presentation only and changing
     * it can never alter what was received.
     */
    /**
     * Whether this follower has a `diaswarm-keys` identity (D26).
     *
     * **OFF BY DEFAULT, AND THE REASON IS THE INVITE.** Having one means
     * handing out a v3 invite — which the subject has to be able to read. An
     * AAPS build older than the field refuses it outright and says so, so
     * turning this on before the person you follow has updated breaks pairing
     * rather than degrading it. Off, this app behaves exactly as it shipped.
     */
    /**
     * When this phone last offered its keys identity to a subject, per subject.
     *
     * **BECAUSE THE OFFER WAS BEING MADE EVERY PASS, FOR EVER.** A handover is
     * a request the subject applies on their side, and nothing told this phone
     * it had landed — so it asked again a minute later, and the subject's phone
     * granted again each time. That is fixed on the receiving side too
     * (`Error::AlreadyGranted`), which is where it has to be fixed, because
     * this phone is not the only thing that can send one.
     *
     * Still throttled rather than stopped: "did it work?" has no reliable
     * answer here, and a handover that is genuinely lost has to be retried or
     * the pairing never completes. Half an hour is slow enough to be free and
     * quick enough that nobody waits for it.
     */
    fun handedOverAt(c: Context, subject: String): Long =
        p(c).getLong(HANDED_OVER_AT + subject, 0L)

    fun setHandedOverAt(c: Context, subject: String, at: Long) =
        p(c).edit().putLong(HANDED_OVER_AT + subject, at).apply()

    /**
     * Read the keys vault and **nothing else** — the cutover, actually tested.
     *
     * **BECAUSE OTHERWISE THERE IS NOTHING TO TEST.** Every reader merges the
     * two vaults, which is right during a migration and is also why "the keys
     * vault can stand alone" has never been checked: the core vault has been
     * quietly covering for it at every step. A gap in the new one is invisible
     * while the old one is still there, which is the exact condition that hid
     * three half-built readers.
     *
     * So this turns the old one off. Anything the keys vault cannot answer
     * shows as missing, immediately and on screen, instead of the day the core
     * vault is retired.
     *
     * Off by default, and useless without [keysVault] — a phone with neither
     * source would show nothing at all, which is not a test, it is a blank
     * screen. Gated on both.
     */
    fun keysOnly(c: Context): Boolean = keysVault(c) && p(c).getBoolean(KEYS_ONLY, false)
    fun setKeysOnly(c: Context, v: Boolean) = p(c).edit().putBoolean(KEYS_ONLY, v).apply()

    /**
     * Whether this phone holds the wifi radio up so it stays reachable asleep.
     *
     * **A CHOICE, BECAUSE IT SPENDS SOMEBODY'S BATTERY.** Deep doze closes the
     * relay connection within about half a minute, and a phone that is not
     * connected to a relay cannot be reached through one — so without this a
     * follower goes quiet whenever the screen has been off for a while, and
     * catches up in a rush when it wakes. With it, the radio stays associated
     * and the readings keep arriving.
     *
     * **OFF by default, and that is a change of mind with a measurement behind
     * it.** It was on, when the mechanism was a `WifiLock` that cost battery
     * and did nothing. What it takes to actually work is a foreground service
     * with a permanent notification, and turning that on for everybody who
     * updates — without asking, without explaining — is not a default anybody
     * should choose on somebody else's behalf.
     *
     * Two hours unplugged said what it is worth: with this off, a sleeping
     * phone made no fetch for forty minutes and the readings arrived in a rush
     * when it woke. Neither answer is wrong. It is a notification and some
     * battery against knowing now, and the person holding the phone is the one
     * who should pick.
     */
    fun stayReachable(c: Context): Boolean = p(c).getBoolean(STAY_REACHABLE, false)
    fun setStayReachable(c: Context, v: Boolean) =
        p(c).edit().putBoolean(STAY_REACHABLE, v).apply()

    /**
     * When the endpoint was last re-established because replication stalled.
     *
     * Persisted rather than held in memory: the process restarts, and a
     * reconnect rate limit that forgets itself on every restart is not a rate
     * limit.
     */
    fun lastEndpointRestart(c: Context): Long = p(c).getLong(LAST_RESTART, 0L)
    fun setLastEndpointRestart(c: Context, at: Long) =
        p(c).edit().putLong(LAST_RESTART, at).apply()

    fun keysVault(c: Context): Boolean = p(c).getBoolean(KEYS_VAULT, false)
    fun setKeysVault(c: Context, v: Boolean) = p(c).edit().putBoolean(KEYS_VAULT, v).apply()

    fun mmol(c: Context): Boolean = p(c).getBoolean(UNITS_MMOL, true)
    fun setMmol(c: Context, v: Boolean) = p(c).edit().putBoolean(UNITS_MMOL, v).apply()

    /** mg/dL to whatever the screen is showing. */
    fun show(c: Context, mgdl: Double): String =
        if (mmol(c)) String.format("%.1f", mgdl / 18.0) else "${mgdl.toInt()}"

    fun unitLabel(c: Context): String = if (mmol(c)) "mmol/L" else "mg/dL"

    /**
     * The range the CHART colours against — this phone's setting, not theirs.
     *
     * **IT IS NOT THE SUBJECT'S TARGET, AND THE UI MUST NEVER IMPLY IT IS.**
     * Their published target may be a single number — this project's own
     * reference subject loops to 7.0–7.0 — and a single point classifies
     * nothing: colouring against it called every reading either high or low,
     * and painted a perfectly ordinary day red.
     *
     * AAPS colours against the user's *display* low and high lines, which are a
     * display preference and are deliberately not in the record vocabulary, so
     * a follower cannot know them. This is the viewer's own substitute, and it
     * is shown on screen labelled as such.
     *
     * The default is **3.9–10.0 mmol/L (70–180 mg/dL)**: the ADA/ATTD consensus
     * time-in-range band, which is a published standard rather than a number
     * invented here. Stored in mg/dL like every other glucose value, so the
     * units toggle stays presentation-only.
     */
    fun lowLine(c: Context): Double = p(c).getFloat(LOW_LINE, 70f).toDouble()
    fun highLine(c: Context): Double = p(c).getFloat(HIGH_LINE, 180f).toDouble()

    fun setLines(c: Context, lowMgdl: Double, highMgdl: Double) =
        p(c).edit()
            .putFloat(LOW_LINE, lowMgdl.toFloat())
            .putFloat(HIGH_LINE, highMgdl.toFloat())
            .apply()

    /**
     * The bands this phone will colour against, and where each comes from.
     *
     * **NONE OF THEM IS INVENTED HERE**, which is the only reason a viewer is
     * allowed to pick one at all. The first two are the ADA/ATTD consensus
     * time-in-range and time-in-tight-range bands; the third is no colouring,
     * which is the honest position when nobody wants to assert a threshold.
     */
    val BANDS: List<Pair<Double, Double>> = listOf(
        70.0 to 180.0,   // consensus time in range, 3.9-10.0
        70.0 to 140.0,   // consensus tight range,   3.9-7.8
        0.0 to 0.0       // off: readingColor returns neutral on a zero-width band
    )

    const val DEFAULT_LOW_MGDL = 70.0
    const val DEFAULT_HIGH_MGDL = 180.0

    /** Move to the next band, wrapping. The only way these are ever set. */
    fun cycleBand(c: Context) {
        val now = lowLine(c) to highLine(c)
        val i = BANDS.indexOfFirst { kotlin.math.abs(it.first - now.first) < 0.5 &&
            kotlin.math.abs(it.second - now.second) < 0.5 }
        val next = BANDS[(i + 1).mod(BANDS.size)]
        setLines(c, next.first, next.second)
    }

    /** How the current band reads on screen, in the unit on screen. */
    fun bandLabel(c: Context): String {
        val lo = lowLine(c)
        val hi = highLine(c)
        return if (hi - lo < 0.5) "no colouring"
        else "${show(c, lo)}–${show(c, hi)} ${unitLabel(c)}"
    }
}
