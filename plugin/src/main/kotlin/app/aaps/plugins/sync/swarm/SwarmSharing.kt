package app.aaps.plugins.sync.swarm

import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import android.graphics.Bitmap
import com.google.zxing.BarcodeFormat
import com.google.zxing.EncodeHintType
import com.google.zxing.MultiFormatWriter
import android.util.TypedValue
import android.view.Gravity
import android.widget.ImageView
import android.widget.LinearLayout
import android.widget.ScrollView
import android.widget.TextView
import com.google.android.material.dialog.MaterialAlertDialogBuilder
import com.google.zxing.qrcode.decoder.ErrorCorrectionLevel

/**
 * The two dialogs that make sharing a thing a person can do.
 *
 * WHY A DIALOG AND NOT A SCREEN. Everything here is either "show me a thing to
 * hand over" or "show me who can read" — both are read-only, momentary, and
 * belong on top of the settings list rather than replacing it. Nothing in this
 * file writes to the vault; granting goes through the preference the sync pass
 * already acts on, so there is exactly one code path that changes access.
 *
 * AN INVITE IS NOT A SECRET. It carries a public key and an endpoint id, and
 * anyone holding it can fetch ciphertext and open none of it — access still
 * requires the subject to grant that specific reader key. So it is safe to put
 * on screen, photograph, and send over any channel. That is worth stating in
 * the dialog itself, because a QR code full of hex looks like a secret and
 * people treat it like one.
 */
object SwarmSharing {

    private fun dp(context: Context, v: Float) =
        TypedValue.applyDimension(TypedValue.COMPLEX_UNIT_DIP, v, context.resources.displayMetrics).toInt()

    private fun column(context: Context): LinearLayout =
        LinearLayout(context).apply {
            orientation = LinearLayout.VERTICAL
            val p = dp(context, 20f)
            setPadding(p, p, p, p)
            gravity = Gravity.CENTER_HORIZONTAL
        }

    /**
     * Render an invite as a QR big enough to scan and small enough to leave
     * room for everything else.
     *
     * SIZED AGAINST THE SHORT EDGE AT ABOUT HALF, not three quarters. The
     * first version used 3/4 of the smaller screen dimension, which on a
     * 1080x2400 phone is 810px: the code rendered perfectly and pushed the
     * title, the explanation, the invite text and both buttons off the dialog,
     * leaving a QR and nothing else. It looked fine in a screenshot and was
     * unusable — you could not read what it was or copy it.
     *
     * Error correction H: the code gets read off a screen at an angle, in a
     * kitchen, by someone's parent. Losing a quarter of the modules and still
     * decoding is worth the density.
     */
    private fun qr(context: Context, text: String): Bitmap {
        val side = (context.resources.displayMetrics.widthPixels * 0.55f).toInt()
        // ZXING'S OWN ENCODER, NOT A WRAPPER AROUND IT. `zxing-android-embedded`
        // is already here for the scanner and brings `com.google.zxing:core`
        // with it, which is what any QR wrapper would have called anyway.
        //
        // The wrapper it replaces came from jitpack, and F-Droid will not build
        // against jitpack — it builds arbitrary source at an arbitrary commit,
        // so the dependency is not reproducible. One line of encoding is a
        // cheaper price than being unpublishable.
        val hints = mapOf(
            EncodeHintType.ERROR_CORRECTION to ErrorCorrectionLevel.H,
            EncodeHintType.MARGIN to 1
        )
        val matrix = MultiFormatWriter().encode(text, BarcodeFormat.QR_CODE, side, side, hints)
        val on = android.graphics.Color.BLACK
        val off = android.graphics.Color.WHITE
        val pixels = IntArray(matrix.width * matrix.height)
        for (y in 0 until matrix.height) {
            val row = y * matrix.width
            for (x in 0 until matrix.width) pixels[row + x] = if (matrix.get(x, y)) on else off
        }
        return Bitmap.createBitmap(pixels, matrix.width, matrix.height, Bitmap.Config.ARGB_8888)
    }

    /**
     * Everything in these dialogs scrolls.
     *
     * A dialog taller than the screen does not scroll by default — it clips,
     * silently, from the bottom. Small phone, large font, long key: assume it
     * will not fit.
     */
    private fun scrolling(context: Context, content: LinearLayout): ScrollView =
        ScrollView(context).apply { addView(content) }

    /**
     * A dialog builder, and THE CONTEXT TO BUILD ITS CONTENTS FROM.
     *
     * These are not the same context, and using the wrong one produces a
     * failure with no error and no missing widget: the app is dark-themed, a
     * Material dialog is light, and a TextView created from the activity's
     * context inherits white text — which it then draws on a white dialog.
     * Everything is present, laid out, and invisible. Two rounds of "the
     * dialog is clipping" were actually this.
     *
     * `builder.context` is the themed one. Build every view from it.
     */
    private fun dialog(context: Context): MaterialAlertDialogBuilder =
        MaterialAlertDialogBuilder(context, app.aaps.core.ui.R.style.DialogTheme)

    /**
     * Show the invite, or say precisely why there isn't one.
     *
     * The two reasons are different and a person can act on both: nothing has
     * been sealed yet (wait), or this node is not serving (turn the plugin on).
     * "Not available" would be true and useless.
     */
    fun showInvite(context: Context, invite: String, whyNot: String?) {
        val builder = dialog(context)
        val ctx = builder.context
        val view = column(ctx)

        if (invite.isEmpty()) {
            view.addView(TextView(ctx).apply {
                text = whyNot ?: ctx.getString(R.string.swarm_invite_not_ready)
            })
            builder.setTitle(R.string.swarm_show_invite)
                .setView(scrolling(ctx, view))
                .setPositiveButton(android.R.string.ok, null)
                .show()
            return
        }

        view.addView(ImageView(ctx).apply {
            setImageBitmap(qr(ctx, invite))
            adjustViewBounds = true
        })
        view.addView(TextView(ctx).apply {
            setText(R.string.swarm_invite_explain)
            setPadding(0, dp(ctx, 12f), 0, dp(ctx, 8f))
        })
        view.addView(TextView(ctx).apply {
            text = invite
            textSize = 10f
            setTextIsSelectable(true)
        })

        builder.setTitle(R.string.swarm_show_invite)
            .setView(scrolling(ctx, view))
            .setPositiveButton(R.string.swarm_copy) { _, _ ->
                val cb = context.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
                cb.setPrimaryClip(ClipData.newPlainText("diaswarm invite", invite))
            }
            .setNegativeButton(android.R.string.cancel, null)
            .show()
    }

    /**
     * Whose history this phone keeps a copy of, and how fresh each one is.
     *
     * THE AGE IS THE POINT, not the number. A follower's dangerous failure is
     * not an error on screen — it is a reading that looks current and is nine
     * hours old, which §12.3 names as the thing a swarm must not do. So every
     * row says when, and a row with nothing readable says which of the two
     * reasons applies: nothing has arrived, or it has arrived and cannot be
     * opened because they have not shared with you.
     */
    fun showFollowing(context: Context, listing: String) {
        val builder = dialog(context)
        val ctx = builder.context
        val view = column(ctx)
        val rows = listing.lines().filter { it.isNotBlank() }
        if (rows.isEmpty()) {
            view.addView(TextView(ctx).apply { setText(R.string.swarm_following_none) })
        } else {
            val now = System.currentTimeMillis()
            rows.forEach { row ->
                val f = row.split('\t')
                val subject = f.getOrElse(0) { "" }
                val purpose = f.getOrElse(1) { "" }
                val reached = f.getOrElse(2) { "0" } == "1"
                val mgdl = f.getOrElse(3) { "" }.toDoubleOrNull()
                val at = f.getOrElse(4) { "" }.toLongOrNull()

                view.addView(TextView(ctx).apply {
                    text = ctx.getString(R.string.swarm_following_row, purpose, subject)
                    textSize = 11f
                    setTextIsSelectable(true)
                    setPadding(0, dp(ctx, 8f), 0, 0)
                })
                view.addView(TextView(ctx).apply {
                    text = when {
                        mgdl != null && at != null ->
                            ctx.getString(R.string.swarm_following_reading, mgdl, (now - at) / 60_000)
                        reached -> ctx.getString(R.string.swarm_following_unreadable)
                        else -> ctx.getString(R.string.swarm_following_nothing)
                    }
                    setPadding(0, 0, 0, dp(ctx, 8f))
                })
            }
        }
        builder.setTitle(R.string.swarm_following)
            .setView(scrolling(ctx, view))
            .setPositiveButton(android.R.string.ok, null)
            .show()
    }

    /**
     * Who can currently read, from the subject's own private book.
     *
     * The grant log cannot answer this — it is filed under tags that name
     * nobody (D13), which is the point. The book is the only place the mapping
     * exists, it never leaves the device, and this is the only thing that
     * reads it.
     */
    fun showReaders(context: Context, listing: String) {
        val builder = dialog(context)
        val ctx = builder.context
        val view = column(ctx)
        val rows = listing.lines().filter { it.isNotBlank() }
        if (rows.isEmpty()) {
            view.addView(TextView(ctx).apply { setText(R.string.swarm_no_readers) })
        } else {
            rows.forEach { row ->
                val parts = row.split('\t')
                val key = parts.getOrElse(0) { "" }
                val purpose = parts.getOrElse(1) { "" }
                view.addView(TextView(ctx).apply {
                    // The whole key, not a prefix: this is the string you paste
                    // into the withdraw box, and a truncated one cannot be.
                    text = ctx.getString(R.string.swarm_reader_row, purpose, key)
                    textSize = 11f
                    setTextIsSelectable(true)
                    setPadding(0, dp(ctx, 6f), 0, dp(ctx, 6f))
                })
            }
            view.addView(TextView(ctx).apply {
                setText(R.string.swarm_readers_explain)
                setPadding(0, dp(ctx, 12f), 0, 0)
            })
        }
        builder.setTitle(R.string.swarm_readers)
            .setView(scrolling(ctx, view))
            .setPositiveButton(android.R.string.ok, null)
            .show()
    }

    /**
     * Ask before re-reading everything.
     *
     * Not because it is dangerous — nothing is written to AAPS's database and
     * re-sealing is idempotent — but because it is long, and a button that
     * silently occupies a phone for several minutes is one people press twice.
     */
    fun confirmResync(context: Context, onConfirm: () -> Unit) {
        MaterialAlertDialogBuilder(context, app.aaps.core.ui.R.style.DialogTheme)
            .setTitle(R.string.swarm_resync)
            .setMessage(R.string.swarm_resync_confirm)
            .setNegativeButton(android.R.string.cancel, null)
            .setPositiveButton(android.R.string.ok) { _, _ -> onConfirm() }
            .show()
    }
}
