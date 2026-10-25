package nz.diaswarm.follower

import android.content.Context
import android.graphics.Bitmap
import android.graphics.Canvas
import android.graphics.Color
import android.graphics.Paint
import android.graphics.RectF
import com.google.zxing.BarcodeFormat
import com.google.zxing.EncodeHintType
import com.google.zxing.MultiFormatWriter
import com.google.zxing.qrcode.decoder.ErrorCorrectionLevel

/**
 * An invite as a code somebody can point a camera at.
 *
 * zxing's own encoder, not a wrapper: `zxing-android-embedded` is already here
 * for the scanner and brings `com.google.zxing:core` with it. The wrapper this
 * replaces came from jitpack, and F-Droid will not build against jitpack.
 *
 * Error correction H because the code gets read off a screen, at an angle, in a
 * kitchen, by somebody's parent. Losing a quarter of the modules and still
 * decoding is worth the density — and it is also what buys room for the mark in
 * the middle (see [withBadge]).
 */
object Qr {
    fun bitmap(context: Context, text: String): Bitmap? = runCatching {
        val side = (context.resources.displayMetrics.widthPixels * 0.62f).toInt()
        val hints = mapOf(
            EncodeHintType.ERROR_CORRECTION to ErrorCorrectionLevel.H,
            EncodeHintType.MARGIN to 1
        )
        val m = MultiFormatWriter().encode(text, BarcodeFormat.QR_CODE, side, side, hints)
        val px = IntArray(m.width * m.height)
        for (y in 0 until m.height) {
            val row = y * m.width
            for (x in 0 until m.width) px[row + x] = if (m.get(x, y)) Color.BLACK else Color.WHITE
        }
        withBadge(Bitmap.createBitmap(px, m.width, m.height, Bitmap.Config.ARGB_8888))
    }.getOrNull()

    /**
     * Stamp the ayni mark into the middle of the code.
     *
     * A white rounded pad first, so the scanner reads a clean block and not a
     * fringe of half-covered modules; level-H error correction then
     * reconstructs the modules the pad hides. The pad is ~24% of the side —
     * roughly 6% of the area — which sits well inside H's ~30% budget even on a
     * long v4 invite, where the code is densest and the mark covers the fewest
     * modules.
     *
     * The mark is the two-arc ring from `ic_launcher_foreground`, drawn here
     * rather than inflated so there is no drawable to carry: one ring in two
     * 138.8-degree arcs (315 degrees and its half-turn), green on the pad.
     */
    private fun withBadge(src: Bitmap): Bitmap {
        val out = src.copy(Bitmap.Config.ARGB_8888, true) ?: return src
        val w = out.width.toFloat()
        val cx = w / 2f
        val cy = w / 2f
        val canvas = Canvas(out)

        val pad = w * 0.24f
        val half = pad / 2f
        val padRadius = pad * 0.22f
        val white = Paint(Paint.ANTI_ALIAS_FLAG).apply {
            color = Color.WHITE
            style = Paint.Style.FILL
        }
        canvas.drawRoundRect(cx - half, cy - half, cx + half, cy + half, padRadius, padRadius, white)

        val rr = pad * 0.28f
        val ring = Paint(Paint.ANTI_ALIAS_FLAG).apply {
            color = Color.parseColor("#35D08A")
            style = Paint.Style.STROKE
            strokeWidth = pad * 0.10f
            strokeCap = Paint.Cap.ROUND
        }
        val box = RectF(cx - rr, cy - rr, cx + rr, cy + rr)
        canvas.drawArc(box, 315f, 138.8f, false, ring)
        canvas.drawArc(box, 135f, 138.8f, false, ring)
        return out
    }
}
