package app.aaps.plugins.sync.swarm

import android.app.Activity
import android.os.Bundle
import android.widget.Toast
import androidx.activity.result.ActivityResultLauncher
import androidx.appcompat.app.AppCompatActivity
import app.aaps.core.keys.interfaces.Preferences
import app.aaps.plugins.sync.swarm.keys.SwarmStringKey
import com.google.android.material.dialog.MaterialAlertDialogBuilder
import com.journeyapps.barcodescanner.ScanContract
import com.journeyapps.barcodescanner.ScanOptions
import dagger.android.support.DaggerAppCompatActivity
import javax.inject.Inject

/**
 * Point the camera at someone else's invite, and decide what to do with it.
 *
 * WHY AN ACTIVITY OF ITS OWN. A scan is a request/response, and a preference
 * row has nowhere to receive a response — it is inside a fragment inside the
 * settings activity, and plumbing an `ActivityResult` back through that is a
 * lot of moving parts to end up in the same place. This starts the scan, acts
 * on the result itself, and finishes. The settings screen only has to launch
 * it.
 *
 * ONE CODE, TWO POSSIBLE MEANINGS, and this is the part worth getting right.
 * An invite carries the sender's public key and where to reach them. Scanning
 * one can mean either:
 *
 *   * **Follow them** — keep a copy of their history. Costs them nothing and
 *     needs no permission from them; you will hold ciphertext you cannot read
 *     until they grant you.
 *   * **Share with them** — grant that key, so they can read YOURS.
 *
 * They are opposite directions and both are ordinary. Guessing which one was
 * meant would be wrong half the time, and silently doing both would hand out
 * access nobody asked to give — so it asks, every time, naming the key.
 */
class SwarmScanActivity : DaggerAppCompatActivity() {

    @Inject lateinit var preferences: Preferences

    private lateinit var scanner: ActivityResultLauncher<ScanOptions>

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)

        scanner = registerForActivityResult(ScanContract()) { result ->
            val text = result.contents
            if (text.isNullOrBlank()) {
                // Cancelled, or the camera was refused. Not an error worth a
                // dialog — the person pressed back.
                finish()
                return@registerForActivityResult
            }
            onScanned(text.trim())
        }

        if (savedInstanceState == null) {
            scanner.launch(
                ScanOptions().apply {
                    setDesiredBarcodeFormats(ScanOptions.QR_CODE)
                    setPrompt(getString(R.string.swarm_scan_prompt))
                    setBeepEnabled(false)
                    // FOLLOW THE DEVICE. Locked is the library's default and it
                    // starts its capture screen in landscape, so a phone held
                    // upright showed a sideways viewfinder — you have to turn
                    // the phone to scan a code someone is holding upright in
                    // front of you, which is the wrong way round.
                    setOrientationLocked(false)
                }
            )
        }
    }

    private fun onScanned(text: String) {
        SwarmNative.check()
        val parsed = SwarmNative.inviteParse(text)
        if (parsed.isEmpty()) {
            // Says WHAT was wrong with it as far as it can: this was a QR code,
            // it just was not one of ours. "Scan failed" would be untrue.
            MaterialAlertDialogBuilder(this, app.aaps.core.ui.R.style.DialogTheme)
                .setTitle(R.string.swarm_scan)
                .setMessage(R.string.swarm_scan_not_an_invite)
                .setPositiveButton(android.R.string.ok) { _, _ -> finish() }
                .setOnCancelListener { finish() }
                .show()
            return
        }

        val parts = parsed.split('\t')
        val subject = parts.getOrElse(0) { "" }
        val purpose = parts.getOrElse(2) { DataSyncSelectorSwarmImpl.PURPOSE }

        MaterialAlertDialogBuilder(this, app.aaps.core.ui.R.style.DialogTheme)
            .setTitle(R.string.swarm_scanned)
            .setMessage(getString(R.string.swarm_scanned_body, subject, purpose))
            .setPositiveButton(R.string.swarm_scan_follow) { _, _ -> follow(text) }
            .setNeutralButton(R.string.swarm_scan_share) { _, _ -> share(subject) }
            .setNegativeButton(android.R.string.cancel) { _, _ -> finish() }
            .setOnCancelListener { finish() }
            .show()
    }

    /** Keep a copy of their history. */
    private fun follow(invite: String) {
        val store = SwarmPaths.store(this).absolutePath
        val n = SwarmNative.netFollow(store, invite)
        toastAndFinish(
            when {
                n > 0L -> getString(R.string.swarm_now_following)
                n == 0L -> getString(R.string.swarm_already_following)
                else -> getString(R.string.swarm_follow_failed, n)
            }
        )
    }

    /**
     * Grant them, by the same route the settings screen uses.
     *
     * Writes the preference the sync pass acts on rather than touching the
     * vault here. One code path changes access, and it is not on this thread.
     */
    private fun share(readerKey: String) {
        preferences.put(SwarmStringKey.GrantReader, readerKey)
        toastAndFinish(getString(R.string.swarm_will_share))
    }

    private fun toastAndFinish(message: String) {
        Toast.makeText(this, message, Toast.LENGTH_LONG).show()
        setResult(Activity.RESULT_OK)
        finish()
    }
}
