package nz.diaswarm.follower

import androidx.compose.animation.core.Animatable
import androidx.compose.animation.core.FastOutSlowInEasing
import androidx.compose.animation.core.LinearEasing
import androidx.compose.animation.core.tween
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Path
import androidx.compose.ui.graphics.PathMeasure
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import kotlin.math.pow

/** What [ScanTick] confirms: that someone scanned, and their name when the invite carried one. */
data class ScanOutcome(val name: String?)

/**
 * The yes the person holding up the code never used to get.
 *
 * A follower's phone changes the instant its camera reads an invite — but the
 * phone DISPLAYING that invite had nothing to show for it, so whoever was
 * holding it out stood there guessing whether it had worked. This plays on the
 * displaying phone the moment the scan completes (the swarm hands it a new
 * subject to follow — see the detection in [FollowerApp]): a green ring that
 * pops in and a tick that draws itself across it, then fades to the app.
 *
 * Drawn the same way as [SwarmSplash] — an [Animatable] driving a [Canvas], in
 * the brand green — so the two moments feel like one app. It plays once and
 * calls [onDone].
 */
@Composable
fun ScanTick(name: String?, onDone: () -> Unit) {
    // The ring pops with an overshoot; the tick then strokes across it; the
    // whole thing holds, then fades to the app. One timeline, three stages.
    val pop = remember { Animatable(0f) }      // ring scale/draw-in
    val mark = remember { Animatable(0f) }     // tick stroke progress
    val fade = remember { Animatable(1f) }     // overlay opacity at the end

    LaunchedEffect(Unit) {
        pop.animateTo(1f, tween(260, easing = { easeOutBack(it) }))
        mark.animateTo(1f, tween(320, easing = FastOutSlowInEasing))
        kotlinx.coroutines.delay(900)
        fade.animateTo(0f, tween(220, easing = LinearEasing))
        onDone()
    }

    val tint = AyniColor.InRange

    val swallow = remember { MutableInteractionSource() }
    Column(
        Modifier
            .fillMaxSize()
            // Eat taps while it plays, so a tap meant for the app behind does not
            // land on a control under the confirmation.
            .clickable(interactionSource = swallow, indication = null) {}
            .alpha(fade.value)
            // Not fully opaque: the app stays faintly visible underneath, so the
            // tick reads as a layer over the thing it confirms, not a new screen.
            .background(AyniColor.Bg.copy(alpha = 0.88f)),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.Center
    ) {
        Canvas(Modifier.size(120.dp)) {
            val s = size.minDimension
            val c = Offset(s / 2f, s / 2f)
            val p = pop.value

            // The ring: a disc of the tint behind, kept faint, and a solid stroke
            // that grows to full as it pops — so it arrives with weight.
            drawCircle(tint, radius = s * 0.46f * p, center = c, alpha = 0.14f * p)
            drawCircle(
                tint, radius = s * 0.42f, center = c, alpha = p,
                style = Stroke(width = s * 0.055f)
            )

            // The tick, traced by length so it draws itself.
            val stroke = Stroke(width = s * 0.07f, cap = androidx.compose.ui.graphics.StrokeCap.Round)
            val path = Path().apply {
                moveTo(s * 0.33f, s * 0.52f)
                lineTo(s * 0.45f, s * 0.64f)
                lineTo(s * 0.68f, s * 0.38f)
            }
            drawTraced(path, mark.value, tint, stroke)
        }

        Spacer(Modifier.height(18.dp))
        androidx.compose.material3.Text(
            text = if (!name.isNullOrBlank()) "Following $name" else "Connected",
            color = AyniColor.Text1,
            fontSize = 17.sp,
            fontWeight = FontWeight.SemiBold,
            textAlign = TextAlign.Center,
            modifier = Modifier.alpha(mark.value)
        )
        androidx.compose.material3.Text(
            text = "Their glucose will appear here",
            color = AyniColor.Text2,
            fontSize = 13.sp,
            textAlign = TextAlign.Center,
            modifier = Modifier.alpha(mark.value).padding(top = 6.dp, start = 32.dp, end = 32.dp)
        )
    }
}

/** Draw the first [fraction] of [path]'s length — the self-drawing effect. */
private fun androidx.compose.ui.graphics.drawscope.DrawScope.drawTraced(
    path: Path, fraction: Float, color: androidx.compose.ui.graphics.Color, stroke: Stroke
) {
    if (fraction <= 0f) return
    val measure = PathMeasure().apply { setPath(path, false) }
    val dst = Path()
    measure.getSegment(0f, measure.length * fraction.coerceIn(0f, 1f), dst, true)
    drawPath(dst, color, style = stroke)
}

/** Overshoot easing, same curve [SwarmSplash] settles the ring with. */
private fun easeOutBack(x: Float): Float {
    val c1 = 1.70158f; val c3 = c1 + 1f
    return 1f + c3 * (x - 1f).pow(3) + c1 * (x - 1f).pow(2)
}
