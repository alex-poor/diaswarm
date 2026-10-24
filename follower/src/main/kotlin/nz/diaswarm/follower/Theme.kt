package nz.diaswarm.follower

import androidx.compose.ui.graphics.Color
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp

/**
 * The one place Ayni's look is defined.
 *
 * Before this file the palette lived three times over — [FollowerApp], [Dialogs]
 * and [GlucoseChart] each carried their own `Color(0xFF…)` constants, and they
 * had drifted: the in-range green was #34C38F in one and #3ED598 in another, the
 * insulin blue #63A8F0 in one and #5B7BF0 in another. Same meaning, different
 * value, which is most of what read as unfinished. Everything references these
 * now; nothing hardcodes a colour.
 */
object AyniColor {
    // ---- surfaces ----
    val Bg = Color(0xFF0E1116)
    val Card = Color(0xFF171B22)
    val CardRaised = Color(0xFF1E232C)
    val Line = Color(0xFF272D38)

    // ---- text ----
    val Text1 = Color(0xFFE8EAED)
    val Text2 = Color(0xFF9AA3B0)
    val Text3 = Color(0xFF6B7380)

    // ---- glucose urgency (what the hero number wears) ----
    // A severity scale, because alerting is a goal: the colour has to say how
    // much it matters, not just "in or out". Stale overrides all of them.
    val Urgent = Color(0xFFE5484D)   // urgent low
    val Low = Color(0xFFF2683C)
    val InRange = Color(0xFF34C38F)
    val High = Color(0xFFE8C15A)     // a yellow, kept clear of the carbs orange
    val VeryHigh = Color(0xFFE0922F)
    val Stale = Color(0xFF737B88)    // old data, any value

    // ---- treatments ----
    val Insulin = Color(0xFF63A8F0)
    val Carbs = Color(0xFFE0A33F)

    // ---- brand ----
    val Ring = Color(0xFF35D08A)

    // ---- the warning amber, for the two places a screen says
    //      "this will not do what you think" and for a stale age ----
    val Warn = Color(0xFFE8A33D)

    // ---- chart-only palette ----
    // Deliberately NOT the hero's. Inside the graph a saturated green in the
    // middle reads as a verdict; these are muted so the excursions stand out and
    // the in-range stretch recedes. Tuned values, preserved exactly — only their
    // home moved here. See GlucoseChart.stateColor.
    val ChartInk = Color(0xFFE8EAED)     // was `Strong`
    val ChartHigh = Color(0xFFF0A93B)
    val ChartLow = Color(0xFFE5645A)
    val ChartInRange = Color(0xFF3ED598)
    val ChartTargetLine = Color(0xFF5E6B7A)
    val ChartAxis = Color(0xFF7A8290)
    val ChartInsulin = Color(0xFF5B7BF0)
}

/** The type scale, in sp. Named by role so a screen asks for "Caption", not "13". */
object AyniType {
    val Display = 54.sp   // the hero number
    val Title = 20.sp
    val Body = 15.sp
    val BodySmall = 14.sp
    val Caption = 13.sp
    val Overline = 12.sp  // section headings, upper-cased with letter-spacing
    val Micro = 11.sp
}

/** Spacing and corner radii, so padding is chosen from a scale and not by eye. */
object AyniSpace {
    val s1 = 4.dp; val s2 = 8.dp; val s3 = 12.dp; val s4 = 16.dp; val s5 = 18.dp; val s6 = 24.dp
}
object AyniRadius {
    val Row = 10.dp; val Card = 16.dp; val Pill = 22.dp
}

/**
 * The hero number's colour, as a function of the reading and this phone's lines.
 *
 * Replaces the old low/in-range/high `readingColor` with the full severity scale
 * and, crucially, a stale override: data older than [STALE_AFTER_MS] is drawn
 * grey whatever its value, because a confident colour on a reading nobody can
 * vouch for is exactly the failure this app exists to make visible.
 *
 * The urgent-low and very-high lines are fixed clinical thresholds, not the
 * viewer's band: they are where a reading stops being a colour preference and
 * starts being something a carer should be told about. The band-off case (the
 * viewer chose "no colouring") stays neutral — alerting is a separate path and
 * does not depend on the band.
 */
object Glucose {
    const val URGENT_LOW_MGDL = 55.0   // 3.1 mmol/L
    const val VERY_HIGH_MGDL = 250.0   // 13.9 mmol/L
    const val STALE_AFTER_MS = 15L * 60_000L

    fun isStale(at: Long, now: Long = System.currentTimeMillis()) = now - at >= STALE_AFTER_MS

    fun color(mgdl: Double, lowLineMgdl: Double, highLineMgdl: Double, stale: Boolean): Color = when {
        stale -> AyniColor.Stale
        highLineMgdl - lowLineMgdl < 0.5 -> AyniColor.Text1   // band off: no colouring
        mgdl <= URGENT_LOW_MGDL -> AyniColor.Urgent
        mgdl < lowLineMgdl -> AyniColor.Low
        mgdl >= VERY_HIGH_MGDL -> AyniColor.VeryHigh
        mgdl > highLineMgdl -> AyniColor.High
        else -> AyniColor.InRange
    }
}
