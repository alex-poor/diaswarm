package nz.diaswarm.follower

import androidx.compose.foundation.Canvas
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.CornerRadius
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.Path
import androidx.compose.ui.graphics.PathEffect
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.drawscope.DrawScope
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.text.TextMeasurer
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.drawText
import androidx.compose.ui.text.rememberTextMeasurer
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import java.util.Calendar
import java.util.Locale
import kotlin.math.ceil
import kotlin.math.max
import kotlin.math.min
import kotlin.math.sqrt

/**
 * Somebody else's trend line, drawn the way their own phone draws it.
 *
 * **THIS IS A PORT OF `HomeGlucoseChart` FROM THE AAPS FORK, NOT AN IMPRESSION
 * OF IT.** An earlier version of this file was written by eye from a
 * screenshot and got three things wrong that the original had already solved:
 * it smoothed the trace into a prettier curve, it coloured the in-range stretch
 * green, and it drew one basal step per `tbr` record. That last one is the
 * instructive failure — temp basals overlap and supersede each other, so a step
 * per record stacks six translucent boxes on top of each other and implies
 * rates that were never simultaneously in force. The original samples the
 * effective rate instead, and says so in a comment.
 *
 * The layout, weights, colours and reasoning below are the fork's. What differs
 * is only what has to: records arrive in mg/dL and are converted here, the
 * palette is Ayni's own rather than AapsTheme's tokens, and there is no
 * prediction series to draw.
 *
 * **WHAT IS ABSENT, AND THE REASON IS NARROWER THAN IT USED TO SAY.** This
 * comment once claimed insulin, carbs and basal "were never sent and never will
 * be". The emitter drains `bolus`, `carb`, `tbr` and `extbolus` alongside
 * `cgm`; a granted follower has been decrypting them since the first drain and
 * throwing them away, because the only reader filtered to `cgm`.
 *
 * Genuinely absent is **prediction and the loop's own reasoning**:
 * `deviceStatus` and `apsResults` are excluded at the emit boundary and the
 * vocabulary is closed, so the EVENTUAL and IOB shown beside this graph on the
 * subject's phone are not late — they are not coming, and are not invented here.
 */
@Composable
fun GlucoseChart(
    readings: List<Follower.Reading>,
    treatments: List<Follower.Treatment>,
    basal: List<Follower.BasalStep>,
    scheduledBasal: Double,
    target: Pair<Double, Double>?,
    lowLine: Double,
    highLine: Double,
    mmol: Boolean,
    modifier: Modifier = Modifier
) {
    val measurer = rememberTextMeasurer()
    val axisStyle = TextStyle(fontSize = 9.sp, color = Axis)
    val valueStyle = TextStyle(fontSize = 13.sp)

    Box(modifier) {
        Canvas(Modifier.fillMaxSize()) {
            if (readings.size < 2) return@Canvas
            drawChart(
                readings, treatments, basal, scheduledBasal, target,
                lowLine, highLine, mmol, measurer, axisStyle, valueStyle
            )
        }
    }
}

private const val GLUCOSE_WEIGHT = 0.60f   // share of height for the glucose panel
private const val RAIL_WEIGHT = 0.10f      // treatment rail
private const val INSULIN_WEIGHT = 0.30f   // delivered insulin

private fun DrawScope.drawChart(
    readings: List<Follower.Reading>,
    treatments: List<Follower.Treatment>,
    basal: List<Follower.BasalStep>,
    scheduledBasal: Double,
    target: Pair<Double, Double>?,
    lowLineMgdl: Double,
    highLineMgdl: Double,
    mmol: Boolean,
    measurer: TextMeasurer,
    axisStyle: TextStyle,
    valueStyle: TextStyle
) {
    // DISPLAY UNITS FROM HERE DOWN. The fork's chart is handed values already
    // converted; ours arrive in mg/dL because that is what the records carry,
    // so the conversion happens once, here, and every number below is in the
    // unit on screen. The headroom constants scale with it for the same reason
    // — "+2" is two mmol/L and two mg/dL is not the same gesture.
    val k = if (mmol) 1 / 18.0 else 1.0
    val decimals = if (mmol) 1 else 0
    val headroom = if (mmol) 2.0 else 36.0
    val slack = if (mmol) 0.5 else 9.0

    val from = readings.first().at
    val to = readings.last().at
    if (to <= from) return

    val lowMark = lowLineMgdl * k
    val highMark = highLineMgdl * k
    val pts = bucketed(readings, 300_000L).map { it.first to it.second * k }
    val raw = readings.map { it.at to it.mgdl * k }
    val hasDenseScatter = pts.size > 1 && raw.size > pts.size
    val trace = if (pts.size > 1) pts else raw
    if (trace.size < 2) return

    val leftPad = 26.dp.toPx()
    val rightPad = 6.dp.toPx()
    val axisH = 14.dp.toPx()
    val plotW = size.width - leftPad - rightPad
    if (plotW <= 0f) return

    val bodyH = size.height - axisH
    val gTop = 2.dp.toPx()
    val gH = bodyH * GLUCOSE_WEIGHT
    val railY = gTop + gH + bodyH * RAIL_WEIGHT * 0.30f
    val iTop = gTop + gH + bodyH * RAIL_WEIGHT
    val iH = bodyH * INSULIN_WEIGHT

    val span = (to - from).toFloat().coerceAtLeast(1f)
    fun x(t: Long): Float = leftPad + (t - from) / span * plotW

    // Always show the band plus headroom, and grow for excursions.
    val maxReading = raw.maxOf { it.second }
    val gHi = max(highMark + headroom, ceil(maxReading + slack))
    val gLo = min(lowMark - headroom / 2, raw.minOf { it.second } - slack).coerceAtLeast(0.0)
    fun y(v: Double): Float = gTop + ((gHi - v.coerceIn(gLo, gHi)) / (gHi - gLo)).toFloat() * gH

    // ---- target band (behind everything) ----
    // THIS PHONE'S display range, which is what the colours are judged against;
    // see [Prefs.lowLine] for why it is not the subject's target.
    drawRect(
        color = InRangeBand.copy(alpha = 0.08f),
        topLeft = Offset(leftPad, y(highMark)),
        size = Size(plotW, y(lowMark) - y(highMark))
    )

    // ---- the only two gridlines that mean anything clinically ----
    listOf(lowMark, highMark).forEach { v ->
        drawLine(InRangeBand.copy(alpha = 0.22f), Offset(leftPad, y(v)), Offset(size.width - rightPad, y(v)), 1f)
        measurer.label(this, fmt(v, decimals), leftPad - 4.dp.toPx(), y(v), axisStyle, alignEnd = true)
    }
    measurer.label(this, fmt(gHi, 0), leftPad - 4.dp.toPx(), y(gHi) + 4.dp.toPx(), axisStyle, alignEnd = true)

    // The SUBJECT'S own target, which may be a single number and on this
    // project's reference subject is. Drawn unlike the band on purpose:
    // conflating the two once painted an ordinary day red.
    target?.let { (tl, th) ->
        listOf(y(max(th, tl) * k), y(min(th, tl) * k)).distinct().forEach {
            drawLine(
                TargetLine, Offset(leftPad, it), Offset(size.width - rightPad, it),
                strokeWidth = 1f, pathEffect = PathEffect.dashPathEffect(floatArrayOf(5f, 7f))
            )
        }
    }

    // ---- raw scatter: every sensor reading, under the trace ----
    // Only present on a dense (1-minute) source. Kept deliberately faint and
    // drawn UNDER the trace: the real spread stays visible — compression lows,
    // early-wear instability, a failing sensor all show up here — while the line
    // stays readable. Smoothing the trace itself would hide exactly the signal
    // you want when something is wrong.
    if (hasDenseScatter) {
        val r = 1.dp.toPx()
        raw.forEach { (t, v) -> drawCircle(Strong.copy(alpha = 0.28f), radius = r, center = Offset(x(t), y(v))) }
    }

    // ---- area under the trace ----
    run {
        val area = Path().apply {
            moveTo(x(trace.first().first), gTop + gH)
            trace.forEach { lineTo(x(it.first), y(it.second)) }
            lineTo(x(trace.last().first), gTop + gH)
            close()
        }
        drawPath(
            area,
            Brush.verticalGradient(
                0f to Strong.copy(alpha = 0.16f),
                1f to Color.Transparent,
                startY = gTop, endY = gTop + gH
            )
        )
    }

    // ---- trace, segment-tinted; a gap over 20 min is a dropout, not a line ----
    val strokeW = 2.dp.toPx()
    for (i in 1 until trace.size) {
        val a = trace[i - 1]
        val b = trace[i]
        if (b.first - a.first > 20 * 60_000L) continue
        drawLine(
            color = stateColor((a.second + b.second) / 2, lowMark, highMark),
            start = Offset(x(a.first), y(a.second)),
            end = Offset(x(b.first), y(b.second)),
            strokeWidth = strokeW,
            cap = StrokeCap.Round
        )
    }

    // ---- treatment rail ----
    drawLine(Divider, Offset(leftPad, railY), Offset(size.width - rightPad, railY), 1f)
    treatments.forEach { t ->
        when (t.kind) {
            "carb" -> drawCircle(
                Carb.copy(alpha = 0.9f),
                radius = (sqrt(t.value).toFloat() * 0.6f).coerceIn(2.5f, 6f).dp.toPx(),
                center = Offset(x(t.at), railY)
            )

            "bolus", "extbolus" -> {
                // SMB is the loop's own micro-bolus and is drawn lighter, so a
                // meal bolus does not look like twelve automatic ones.
                val smb = t.flag.equals("SMB", ignoreCase = true)
                val h = (t.value.toFloat() * 1.1f).coerceIn(4f, 13f).dp.toPx()
                drawLine(
                    Insulin.copy(alpha = if (smb) 0.65f else 1f),
                    Offset(x(t.at), railY - h / 2), Offset(x(t.at), railY + h / 2),
                    strokeWidth = (if (smb) 1.5f else 2.6f).dp.toPx(),
                    cap = StrokeCap.Round
                )
            }
        }
    }

    // ---- delivered insulin ----
    val iMax = max(basal.maxOfOrNull { it.rate } ?: 0.0, scheduledBasal).coerceAtLeast(0.1) * 1.15
    fun iy(r: Double): Float = iTop + iH - (r.coerceIn(0.0, iMax) / iMax).toFloat() * iH

    if (basal.size > 1) {
        val step = Path().apply {
            moveTo(leftPad, iy(basal.first().rate))
            for (i in 1 until basal.size) {
                val px = x(basal[i].at)
                lineTo(px, iy(basal[i - 1].rate))
                lineTo(px, iy(basal[i].rate))
            }
            lineTo(size.width - rightPad, iy(basal.last().rate))
            lineTo(size.width - rightPad, iTop + iH)
            lineTo(leftPad, iTop + iH)
            close()
        }
        drawPath(
            step,
            Brush.verticalGradient(
                0f to Insulin.copy(alpha = 0.38f),
                1f to Insulin.copy(alpha = 0.06f),
                startY = iTop, endY = iTop + iH
            )
        )
        drawPath(step, Insulin, style = Stroke(width = 1.4.dp.toPx()))
    }

    // Panel label sits INSIDE the panel on its own ground: in the rail band
    // above, it collided with whichever treatment fell near the left edge.
    run {
        val laid = measurer.measure("INSULIN U/HR", axisStyle)
        drawRoundRect(
            Surface.copy(alpha = 0.85f),
            topLeft = Offset(leftPad, iTop + 2.dp.toPx()),
            size = Size(laid.size.width + 6.dp.toPx(), laid.size.height + 2.dp.toPx()),
            cornerRadius = CornerRadius(3.dp.toPx())
        )
        drawText(laid, topLeft = Offset(leftPad + 3.dp.toPx(), iTop + 3.dp.toPx()))
    }

    if (scheduledBasal > 0) {
        drawLine(
            Axis.copy(alpha = 0.8f),
            Offset(leftPad, iy(scheduledBasal)), Offset(size.width - rightPad, iy(scheduledBasal)),
            strokeWidth = 1f, pathEffect = PathEffect.dashPathEffect(floatArrayOf(3f, 4f))
        )
        measurer.label(this, "sched " + fmt(scheduledBasal, 2), leftPad + 3.dp.toPx(),
            iy(scheduledBasal) - 3.dp.toPx(), axisStyle)
    }

    // ---- time axis ----
    val hourMs = 3_600_000L
    val stepH = if (span > 14 * hourMs) 4 else if (span > 7 * hourMs) 2 else 1
    var t = ceilToHour(from)
    while (t <= to) {
        val cal = Calendar.getInstance().apply { timeInMillis = t }
        if (cal.get(Calendar.HOUR_OF_DAY) % stepH == 0)
            measurer.label(this, String.format(Locale.getDefault(), "%02d", cal.get(Calendar.HOUR_OF_DAY)),
                x(t), size.height - 2.dp.toPx(), axisStyle, center = true)
        t += hourMs
    }

    // ---- newest ----
    // The fork calls this "now"; on a follower it is the newest reading that
    // reached this phone, which is not the same thing and is exactly why the
    // card above always states its age.
    val last = trace.last()
    val nowX = x(last.first)
    drawLine(
        Insulin.copy(alpha = 0.45f), Offset(nowX, gTop), Offset(nowX, iTop + iH),
        strokeWidth = 1f, pathEffect = PathEffect.dashPathEffect(floatArrayOf(3f, 4f))
    )
    val stateC = stateColor(last.second, lowMark, highMark)
    drawCircle(stateC.copy(alpha = 0.16f), radius = 7.dp.toPx(), center = Offset(nowX, y(last.second)))
    drawCircle(stateC, radius = 3.4.dp.toPx(), center = Offset(nowX, y(last.second)))
    drawCircle(Surface, radius = 3.4.dp.toPx(), center = Offset(nowX, y(last.second)), style = Stroke(1.4.dp.toPx()))

    val txt = fmt(last.second, decimals)
    val laid = measurer.measure(txt, valueStyle.copy(color = stateC))
    val chipW = laid.size.width + 8.dp.toPx()
    val chipH = laid.size.height + 3.dp.toPx()
    val chipX = (nowX - 10.dp.toPx() - chipW).coerceAtLeast(leftPad)
    val chipY = (y(last.second) - 10.dp.toPx() - chipH).coerceAtLeast(gTop)
    drawRoundRect(
        Surface.copy(alpha = 0.92f), topLeft = Offset(chipX, chipY), size = Size(chipW, chipH),
        cornerRadius = CornerRadius(4.dp.toPx())
    )
    drawText(laid, topLeft = Offset(chipX + 4.dp.toPx(), chipY + 1.5.dp.toPx()))
}

private val Strong = Color(0xFFE8EAED)
private val High = Color(0xFFF0A93B)
private val Low = Color(0xFFE5645A)
private val InRangeBand = Color(0xFF3ED598)

/** `AapsTone.InRange` — the hero's green, which the trace deliberately does not use. */
private val HeroInRange = Color(0xFF3ED598)
private val TargetLine = Color(0xFF5E6B7A)
private val Axis = Color(0xFF7A8290)
private val Divider = Color(0xFF2A3140)
private val Insulin = Color(0xFF5B7BF0)
private val Carb = Color(0xFFE0A33F)
private val Surface = Color(0xFF171B22)

/**
 * The TRACE's colour — in range is the plain strong text ink, not green.
 *
 * **THE CHART AND THE HERO NUMBER SHARE THRESHOLDS AND NOT PALETTES, AND THAT
 * IS DELIBERATE IN THE ORIGINAL.** The fork's chart `stateColor` returns
 * `textOnSurfaceStrong` in range while its hero number takes `AapsTone.InRange`,
 * which is green. Both are judged against the same two marks, so they can never
 * classify a reading differently — only draw it differently. A trace that goes
 * green in the middle reads as a verdict and leaves nothing distinct for the
 * excursions; a big number that does is the at-a-glance "fine" a person opens
 * the app for.
 */
private fun stateColor(v: Double, low: Double, high: Double): Color = when {
    high - low < 0.05 -> Strong
    v < low -> Low
    v > high -> High
    else -> Strong
}

/**
 * The HERO number's colour, from the same thresholds — green in range.
 *
 * Takes mg/dL, because everything outside this file holds mg/dL. See
 * [stateColor] for why this is not the same palette.
 */
fun readingColor(mgdl: Double, lowLineMgdl: Double, highLineMgdl: Double): Color = when {
    highLineMgdl - lowLineMgdl < 0.5 -> Strong
    mgdl < lowLineMgdl -> Low
    mgdl > highLineMgdl -> High
    else -> HeroInRange
}

/**
 * The 5-minute bucketed series — what the loop's own statistics are computed
 * from, so the line and the decisions cannot disagree.
 *
 * **NOT A COSMETIC FILTER.** It is the same averaging the loop uses; the raw
 * readings stay on screen underneath precisely so nothing is hidden by it.
 */
private fun bucketed(readings: List<Follower.Reading>, bucketMs: Long): List<Pair<Long, Double>> {
    if (readings.isEmpty()) return emptyList()
    val out = ArrayList<Pair<Long, Double>>()
    var i = 0
    while (i < readings.size) {
        val start = readings[i].at
        var sum = 0.0
        var n = 0
        var lastAt = start
        while (i < readings.size && readings[i].at - start < bucketMs) {
            sum += readings[i].mgdl
            lastAt = readings[i].at
            n++
            i++
        }
        if (n > 0) out.add(((start + lastAt) / 2) to (sum / n))
    }
    return out
}

private fun fmt(v: Double, decimals: Int): String =
    if (decimals <= 0) String.format(Locale.getDefault(), "%.0f", v)
    else String.format(Locale.getDefault(), "%.${decimals}f", v)

private fun ceilToHour(t: Long): Long = (t / 3_600_000L + 1) * 3_600_000L

/** Draw a short axis/caption label; [y] is the text BASELINE. */
private fun TextMeasurer.label(
    scope: DrawScope,
    text: String,
    x: Float,
    y: Float,
    style: TextStyle,
    alignEnd: Boolean = false,
    center: Boolean = false
) {
    val laid = measure(text, style)
    val dx = when {
        alignEnd -> x - laid.size.width
        center -> x - laid.size.width / 2f
        else -> x
    }
    // Clip labels that would spill outside the canvas rather than letting them
    // overlap the edge.
    if (dx < -1f || dx + laid.size.width > scope.size.width + 1f) return
    scope.drawText(laid, topLeft = Offset(dx, y - laid.size.height))
}
