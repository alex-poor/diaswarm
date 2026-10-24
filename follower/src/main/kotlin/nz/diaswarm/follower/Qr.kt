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
 * Error correction L, NOT H — and this was the bug that stopped it scanning.
 * The invite is ~720 bytes (keys + endpoint + handle), which at H needs QR
 * version ~29 (≈137 modules a side): rendered at ~0.6 of a phone's width that
 * is ~4px a module, below what another phone's camera resolves screen-to-screen,
 * so it never decoded in either direction. H's resilience buys nothing on a
 * glowing screen held still for a one-time pairing; the density it costs is what
 * made the code unreadable. L drops the same payload to version ~19 (≈93
 * modules), ~6px a module — the difference between "won't scan" and "scans".
 * Quiet zone 4 modules, the spec minimum: the old 1 left scanners unable to find
 * the finder patterns, which compounded it.
 */
object Qr {
    fun bitmap(context: Context, text: String): Bitmap? = runCatching {
        val side = (context.resources.displayMetrics.widthPixels * 0.62f).toInt()
        val hints = mapOf(
            EncodeHintType.ERROR_CORRECTION to ErrorCorrectionLevel.L,
            EncodeHintType.MARGIN to 4
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
