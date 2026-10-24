package nz.diaswarm.follower

import androidx.compose.animation.core.Animatable
import androidx.compose.animation.core.LinearEasing
import androidx.compose.animation.core.tween
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.remember
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.unit.dp
import kotlin.math.PI
import kotlin.math.cos
import kotlin.math.pow
import kotlin.math.sin
import kotlin.random.Random

/**
 * The startup animation: a swarm of disparate dots, scattered and drifting,
 * that gather into the ring the launcher icon draws — many separate things
 * becoming one, which is the whole idea of the app.
 *
 * It plays once per cold start, then [onDone] hands the screen to the app. The
 * geometry is the real logo: two 138.8-degree arcs of a radius-26 ring centred
 * in a 108 box (see ic_launcher_foreground.xml), so the dots settle onto
 * exactly the mark you see on the home screen.
 *
 * Numbers below were chosen in a browser preview and are the ones that looked
 * right: no throb, a tight 14-unit scatter, dots starting at 1.25 and growing
 * to the stroke's half-width, a scattered phase that runs to the halfway mark.
 */

private const val DURATION_MS = 1900
private const val SCATTER_END = 0.5f       // fraction disparate before converging
private const val INITIAL_RADIUS = 1.25f   // dot radius at the start, in box units
private const val FINAL_RADIUS = 4.5f      // == half the 9-unit logo stroke
private const val SCATTER = 14f            // how far dots start from home, in box units
private const val BOX_DP = 200             // the 108-unit box, this many dp on screen

private val RING = Color(0xFF35D08A)
private val BACKDROP = Color(0xFF12161C)   // matches windowSplashScreenBackground

private class Dot(
    val hx: Float, val hy: Float,          // home: where it settles on the ring
    val sx: Float, val sy: Float,          // scattered start
    val driftA: Float, val driftR: Float,  // slow wander while disparate
)

/** The two arcs as centre angles: start (degrees) and a shared 138.8-degree sweep. */
private fun ring(rng: Random): List<Dot> {
    val cx = 54f; val cy = 54f; val r = 26f
    val sweep = 138.8f
    val stepDeg = Math.toDegrees((2.3f / r).toDouble()).toFloat()  // ~2.3-unit spacing
    val n = (sweep / stepDeg).toInt()
    val dots = ArrayList<Dot>()
    for (start in floatArrayOf(-45f, 135f)) {                      // arc, and its half-turn
        for (i in 0..n) {
            val a = Math.toRadians((start + sweep * i / n).toDouble())
            val hx = cx + r * cos(a).toFloat()
            val hy = cy + r * sin(a).toFloat()
            val sa = rng.nextFloat() * 2f * PI.toFloat()
            val dist = SCATTER * (0.4f + 0.6f * rng.nextFloat())
            dots += Dot(
                hx, hy,
                hx + cos(sa) * dist, hy + sin(sa) * dist,
                rng.nextFloat() * 2f * PI.toFloat(),
                2f + rng.nextFloat() * 3f,
            )
        }
    }
    return dots
}

private fun easeOutBack(x: Float): Float {
    val c1 = 1.70158f; val c3 = c1 + 1f
    return 1f + c3 * (x - 1f).pow(3) + c1 * (x - 1f).pow(2)
}

@Composable
fun SwarmSplash(onDone: () -> Unit) {
    val dots = remember { ring(Random(System.nanoTime())) }
    val progress = remember { Animatable(0f) }
    val fade = remember { Animatable(1f) }

    LaunchedEffect(Unit) {
        progress.animateTo(1f, tween(DURATION_MS, easing = LinearEasing))
        fade.animateTo(0f, tween(220, easing = LinearEasing))   // cross-fade to the app
        onDone()
    }

    val boxPx = with(LocalDensity.current) { BOX_DP.dp.toPx() }

    Canvas(
        Modifier
            .fillMaxSize()
            .alpha(fade.value)
            .background(BACKDROP)
    ) {
        val t = progress.value
        val elapsed = t * DURATION_MS
        // Hold scattered, then converge with a gentle overshoot.
        val cv = if (t <= SCATTER_END) 0f else easeOutBack((t - SCATTER_END) / (1f - SCATTER_END))

        val scale = boxPx / 108f
        val ox = (size.width - boxPx) / 2f
        val oy = (size.height - boxPx) / 2f
        val radius = INITIAL_RADIUS + (FINAL_RADIUS - INITIAL_RADIUS) * cv
        val alpha = (0.5f + 0.5f * cv).coerceIn(0.1f, 1f)

        for (d in dots) {
            val drift = (1f - cv) * sin(elapsed / 420f + d.driftA) * d.driftR
            val ax = d.sx + cos(d.driftA) * drift
            val ay = d.sy + sin(d.driftA) * drift
            val x = ox + (ax + (d.hx - ax) * cv) * scale
            val y = oy + (ay + (d.hy - ay) * cv) * scale
            drawCircle(RING, radius * scale, Offset(x, y), alpha = alpha)
        }
    }
}
