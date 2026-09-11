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
    fun mmol(c: Context): Boolean = p(c).getBoolean(UNITS_MMOL, true)
    fun setMmol(c: Context, v: Boolean) = p(c).edit().putBoolean(UNITS_MMOL, v).apply()

    /** mg/dL to whatever the screen is showing. */
    fun show(c: Context, mgdl: Double): String =
        if (mmol(c)) String.format("%.1f", mgdl / 18.0) else "${mgdl.toInt()}"

    fun unitLabel(c: Context): String = if (mmol(c)) "mmol/L" else "mg/dL"
}
