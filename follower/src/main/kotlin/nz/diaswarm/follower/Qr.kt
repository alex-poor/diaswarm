package nz.diaswarm.follower

import android.content.Context
import android.graphics.Bitmap
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
 * decoding is worth the density.
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
            for (x in 0 until m.width) px[row + x] = if (m.get(x, y)) android.graphics.Color.BLACK else android.graphics.Color.WHITE
        }
        Bitmap.createBitmap(px, m.width, m.height, Bitmap.Config.ARGB_8888)
    }.getOrNull()
}
