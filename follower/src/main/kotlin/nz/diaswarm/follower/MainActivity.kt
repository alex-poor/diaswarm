package nz.diaswarm.follower

import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.core.splashscreen.SplashScreen.Companion.installSplashScreen
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.work.*
import com.journeyapps.barcodescanner.ScanContract
import com.journeyapps.barcodescanner.ScanOptions
import java.util.concurrent.TimeUnit

/**
 * The whole app: one screen showing one person's glucose.
 *
 * No tabs, no plugins, no configuration beyond who to watch. Everything a loop
 * needs and a follower does not is not hidden here — it was never written.
 */
class MainActivity : ComponentActivity() {

    private var scanned by mutableStateOf<String?>(null)

    private val scanner = registerForActivityResult(ScanContract()) { result ->
        result.contents?.trim()?.takeIf { it.isNotEmpty() }?.let { scanned = it }
    }

    // The notification grant for glucose alerts. Asked once, from onCreate; a
    // refusal is reconsidered in the Alerts settings group, not re-prompted here.
    private val askNotifications =
        registerForActivityResult(ActivityResultContracts.RequestPermission()) { }

    override fun onCreate(savedInstanceState: Bundle?) {
        // Before super.onCreate: swaps the launch theme for the real one. The
        // system splash is just a plain #12161C (transparent icon), dismissed
        // at once, so the Compose SwarmSplash below is the first thing drawn and
        // there is no launcher-icon flash before it.
        installSplashScreen()
        super.onCreate(savedInstanceState)
        Endpoint.start(applicationContext)
        // Bring the service into line with the preference on every launch:
        // Android may have killed it, and the user may have changed their mind
        // in the system settings rather than in here.
        StayAwake.apply(applicationContext)
        schedule()
        maybeAskNotifications()
        setContent {
            // The app composes underneath immediately (so onResume's sync is
            // already in flight); the swarm plays over it once, then reveals it.
            Box(Modifier.fillMaxSize()) {
                FollowerApp(onScan = { scanner.launch(scanOptions()) }, scanned = scanned, onScanHandled = { scanned = null })
                var splashDone by rememberSaveable { mutableStateOf(false) }
                if (!splashDone) SwarmSplash(onDone = { splashDone = true })
            }
        }
    }

    override fun onResume() {
        super.onResume()
        // Ask now rather than waiting for the poll: somebody opening the app is
        // the one moment they are definitely waiting for an answer.
        Sync.now(this)
    }

    /**
     * Ask for the notification grant once, the first launch where alerts are on
     * and it is missing.
     *
     * Only on 33+, where it is a runtime permission; below that it is granted at
     * install. Gated on [Prefs.askedNotifications] so it is a single prompt, not
     * one on every open — and gated on [Alerts.anyEnabled] so a person who has
     * turned every alert off is not asked for a permission nothing will use.
     */
    private fun maybeAskNotifications() {
        if (android.os.Build.VERSION.SDK_INT < android.os.Build.VERSION_CODES.TIRAMISU) return
        if (Prefs.askedNotifications(this) || !Alerts.anyEnabled(this)) return
        if (checkSelfPermission(android.Manifest.permission.POST_NOTIFICATIONS) ==
            android.content.pm.PackageManager.PERMISSION_GRANTED) return
        Prefs.setAskedNotifications(this)
        askNotifications.launch(android.Manifest.permission.POST_NOTIFICATIONS)
    }

    private fun scanOptions() = ScanOptions().apply {
        setDesiredBarcodeFormats(ScanOptions.QR_CODE)
        setPrompt("Point the camera at their invite")
        setBeepEnabled(false)
        // FOLLOW THE DEVICE. Locked is the library's default and starts the
        // viewfinder sideways, so you would have to turn the phone to scan a
        // code somebody is holding upright in front of you.
        setOrientationLocked(false)
    }

    /**
     * Fifteen minutes is WorkManager's floor for periodic work, which is far too
     * slow for somebody watching glucose — so the periodic job exists only to
     * restart the chain after the process is killed, and [SyncWorker] is asked
     * again on every resume.
     */
    private fun schedule() {
        WorkManager.getInstance(this).enqueueUniquePeriodicWork(
            SyncWorker.UNIQUE,
            ExistingPeriodicWorkPolicy.KEEP,
            PeriodicWorkRequestBuilder<SyncWorker>(15, TimeUnit.MINUTES).build()
        )
    }
}
