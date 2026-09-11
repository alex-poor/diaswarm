package nz.diaswarm.follower

import androidx.compose.foundation.Canvas
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.Path
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.unit.dp
import kotlin.math.max
import kotlin.math.min

/**
 * Somebody else's trend line, and nothing else on the plot.
 *
 * **WHAT IS DELIBERATELY ABSENT.** No insulin, no carbs, no basal, no
 * predictions. §4 excludes loop telemetry from what is shared, so on a follower
 * those are not "not yet loaded" — they were never sent and never will be.
 * Drawing an empty insulin axis under a label would state a zero nobody
 * measured, and this app is in no position to make that claim.
 *
 * The target band is the SUBJECT'S, read from their own published profile. A
 * band drawn from a locally invented default would be a clinical statement made
 * on their behalf by a phone that knows nothing about them.
 */
@Composable
fun GlucoseChart(
    readings: List<Follower.Reading>,
    target: Pair<Double, Double>?,
    mmol: Boolean,
    modifier: Modifier = Modifier
) {
    val line = Color(0xFFE8EAED)
    val band = Color(0xFF34C38F)
    val axis = Color(0xFF7A8290)

    Box(modifier.fillMaxWidth().height(260.dp)) {
        Canvas(Modifier.fillMaxWidth().height(260.dp)) {
            if (readings.size < 2) return@Canvas

            val from = readings.first().at
            val to = readings.last().at
            if (to <= from) return@Canvas

            // The axis covers the readings AND the band, so a line that leaves
            // the band is visibly outside it rather than clipped to the edge.
            val lo = min(readings.minOf { it.mgdl }, target?.first ?: Double.MAX_VALUE) - 18
            val hi = max(readings.maxOf { it.mgdl }, target?.second ?: Double.MIN_VALUE) + 18
            if (hi <= lo) return@Canvas

            val padL = 44f
            val padR = 8f
            val padT = 10f
            val padB = 22f
            val w = size.width - padL - padR
            val h = size.height - padT - padB

            fun x(t: Long) = padL + ((t - from).toFloat() / (to - from).toFloat()) * w
            fun y(v: Double) = padT + (1f - ((v - lo) / (hi - lo)).toFloat()) * h

            target?.let { (tl, th) ->
                val top = y(max(th, tl))
                val bottom = y(min(th, tl))
                // A band of zero height still has to be visible: a subject whose
                // low and high targets are equal has a LINE, not nothing.
                val height = max(bottom - top, 1.5f)
                drawRect(
                    band.copy(alpha = 0.13f),
                    topLeft = Offset(padL, top),
                    size = androidx.compose.ui.geometry.Size(w, height)
                )
            }

            val path = Path().apply {
                moveTo(x(readings.first().at), y(readings.first().mgdl))
                readings.drop(1).forEach { lineTo(x(it.at), y(it.mgdl)) }
            }
            drawPath(path, line, style = Stroke(width = 3f))

            // The most recent reading, marked, because it is the one a person
            // is actually looking for.
            readings.last().let { drawCircle(line, radius = 6f, center = Offset(x(it.at), y(it.mgdl))) }

            drawLine(axis.copy(alpha = 0.3f), Offset(padL, padT + h), Offset(padL + w, padT + h))
        }
    }
}
