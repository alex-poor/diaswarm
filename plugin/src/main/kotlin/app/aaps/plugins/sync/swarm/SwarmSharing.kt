package app.aaps.plugins.sync.swarm

import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import android.graphics.Bitmap
import com.google.zxing.BarcodeFormat
import com.google.zxing.EncodeHintType
import com.google.zxing.MultiFormatWriter
import app.aaps.core.ui.dialogs.OKDialog
import app.aaps.core.ui.extensions.scanForActivity
import app.aaps.plugins.sync.swarm.compose.FollowingSheet
import app.aaps.plugins.sync.swarm.compose.InviteSheet
import app.aaps.plugins.sync.swarm.compose.ReadersSheet
import app.aaps.plugins.sync.swarm.compose.ReadersSheetStrings
import app.aaps.plugins.sync.swarm.compose.SwarmFollowingRow
import app.aaps.plugins.sync.swarm.compose.SwarmReaderRow
import app.aaps.plugins.sync.swarm.compose.SwarmSheetDialog
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

    /**
     * Render an invite as a QR a camera on another phone can actually read.
     *
     * **NO CENTRE BADGE, AND THAT IS THE FIX.** The brand ring was stamped into
     * the middle of the code on the theory that error correction rebuilds the
     * modules the pad hides. That holds at level H — which the follower's own
     * code uses — but this invite carries a p2panda key bundle and is far too
     * dense at H to read screen-to-screen, so it is encoded at level L. L has
     * almost no budget to spare (~7%), and a pad ~24% of the side (≈6% of the
     * area) consumed all of it: a follower could not decode this code at all.
     * The plain code scans; the badge was costing the entire feature. (The
     * follower keeps its badge because it encodes at H and can afford it.)
     *
     * Error correction L because even the compact v5 (base64url) invite is a
     * few hundred bytes with the bundle, and H would push it past ~v29 (≈137
     * modules) — unreadable off a screen. Quiet zone 4 (spec minimum); the old
     * margin 1 left scanners unable to find the finder patterns.
     *
     * Sized at 0.9 of the short edge so the modules stay crisp once
     * [InviteSheet] draws the code at full width; a source smaller than the
     * display was upscaled and softened every module.
     */
    private fun qr(context: Context, text: String): Bitmap {
        val side = (context.resources.displayMetrics.widthPixels * 0.9f).toInt()
        // ZXING'S OWN ENCODER, NOT A WRAPPER AROUND IT. `zxing-android-embedded`
        // is already here for the scanner and brings `com.google.zxing:core`
        // with it, which is what any QR wrapper would have called anyway.
        //
        // The wrapper it replaces came from jitpack, and F-Droid will not build
        // against jitpack — it builds arbitrary source at an arbitrary commit,
        // so the dependency is not reproducible. One line of encoding is a
        // cheaper price than being unpublishable.
        val hints = mapOf(
            EncodeHintType.ERROR_CORRECTION to ErrorCorrectionLevel.L,
            EncodeHintType.MARGIN to 4
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
     * Show the invite, or say precisely why there isn't one.
     *
     * The two reasons are different and a person can act on both: nothing has
     * been sealed yet (wait), or this node is not serving (turn the plugin on).
     * "Not available" would be true and useless.
     */
    fun showInvite(context: Context, invite: String, whyNot: String?) {
        val activity = context.scanForActivity() ?: return
        // The QR is the thing to hand over; "Copy code" puts the full text on the
        // clipboard for the channels a camera cannot reach. No QR when there is
        // no invite — just the plain reason why.
        val qr = if (invite.isEmpty()) null else qr(context, invite)
        val why = whyNot ?: context.getString(R.string.swarm_invite_not_ready)
        SwarmSheetDialog.show(activity, "swarmInvite") { dismiss ->
            InviteSheet(
                title = context.getString(R.string.swarm_show_invite),
                qr = qr,
                whyNot = why,
                explain = context.getString(R.string.swarm_invite_explain),
                copyLabel = context.getString(R.string.swarm_copy),
                onCopy = {
                    val cb = context.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
                    cb.setPrimaryClip(ClipData.newPlainText("diaswarm invite", invite))
                    dismiss()
                },
                onClose = dismiss
            )
        }
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
        val activity = context.scanForActivity() ?: return
        val now = System.currentTimeMillis()
        val rows = listing.lines().filter { it.isNotBlank() }.map { row ->
            val f = row.split('\t')
            val subject = f.getOrElse(0) { "" }
            val purpose = f.getOrElse(1) { "" }
            val reached = f.getOrElse(2) { "0" } == "1"
            val mgdl = f.getOrElse(3) { "" }.toDoubleOrNull()
            val at = f.getOrElse(4) { "" }.toLongOrNull()
            // THE AGE IS THE POINT: a row with nothing readable says which of the
            // two reasons applies — nothing arrived, or it arrived and will not
            // open because they have not shared back.
            val status = when {
                mgdl != null && at != null -> context.getString(R.string.swarm_following_reading, mgdl, (now - at) / 60_000)
                reached -> context.getString(R.string.swarm_following_unreadable)
                else -> context.getString(R.string.swarm_following_nothing)
            }
            SwarmFollowingRow(context.getString(R.string.swarm_following_row, purpose, subject), status)
        }
        SwarmSheetDialog.show(activity, "swarmFollowing") { dismiss ->
            FollowingSheet(
                title = context.getString(R.string.swarm_following),
                rows = rows,
                empty = context.getString(R.string.swarm_following_none),
                onClose = dismiss
            )
        }
    }

    /**
     * Who can currently read, from the subject's own private book.
     *
     * The grant log cannot answer this — it is filed under tags that name
     * nobody (D13), which is the point. The book is the only place the mapping
     * exists, it never leaves the device, and this is the only thing that
     * reads it.
     */
    fun showReaders(context: Context, listing: String, onRevoke: (String) -> Unit) {
        // **A BOTTOM SHEET IN THE APP'S OWN DESIGN SYSTEM, NOT A LEGACY DIALOG.**
        // Built from SheetSurface/ListRow (see ReadersSheet), hosted by a
        // DialogFragment, so it needs a FragmentManager — which the preference's
        // own context scans to. If there is no activity in the chain there is
        // nothing to attach a sheet to, so there is nothing to show.
        val activity = context.scanForActivity() ?: return
        val rows = listing.lines().filter { it.isNotBlank() }.map { row ->
            val parts = row.split('\t')
            val key = parts.getOrElse(0) { "" }
            val purpose = parts.getOrElse(1) { "" }
            // Empty scope means nobody wrote it down — unknown, not unlimited.
            val scope = when (val days = parts.getOrElse(2) { "" }.toLongOrNull()) {
                null -> context.getString(R.string.swarm_reader_scope_unknown)
                0L   -> context.getString(R.string.swarm_reader_scope_all)
                else -> context.resources.getQuantityString(R.plurals.swarm_reader_scope_days, days.toInt(), days)
            }
            // A name if they gave one; a short key as a label if they did not.
            val name = parts.getOrElse(3) { "" }
            val label = if (name.isNotEmpty()) name else context.getString(R.string.swarm_reader_unnamed, key.take(12))
            SwarmReaderRow(key, label, "$purpose · $scope")
        }
        val strings = ReadersSheetStrings(
            title = context.getString(R.string.swarm_readers),
            empty = context.getString(R.string.swarm_no_readers),
            footer = context.getString(R.string.swarm_readers_explain),
            stop = context.getString(R.string.swarm_reader_stop),
            confirmTitle = context.getString(R.string.swarm_reader_stop_confirm),
            explain = context.getString(R.string.swarm_revoke_explain)
        )
        SwarmSheetDialog.show(activity, "swarmReaders") { dismiss ->
            ReadersSheet(rows, strings, onStop = { r -> onRevoke(r.key) }, onClose = dismiss)
        }
    }

    /**
     * Ask before re-reading everything.
     *
     * Not because it is dangerous — nothing is written to AAPS's database and
     * re-sealing is idempotent — but because it is long, and a button that
     * silently occupies a phone for several minutes is one people press twice.
     */
    fun confirmResync(context: Context, onConfirm: () -> Unit) {
        // OKDialog already renders the new Compose confirm (AlertContent); no
        // custom view, so it needs no sheet of its own.
        val activity = context.scanForActivity()
        if (activity != null)
            OKDialog.showConfirmation(
                activity,
                context.getString(R.string.swarm_resync),
                context.getString(R.string.swarm_resync_confirm),
                Runnable { onConfirm() }
            )
        else onConfirm()
    }
}
