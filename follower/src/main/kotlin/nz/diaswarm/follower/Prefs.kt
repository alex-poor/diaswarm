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
