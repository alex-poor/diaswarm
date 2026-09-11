package nz.diaswarm.follower

import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
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

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        Endpoint.start(applicationContext)
        schedule()
        setContent { FollowerApp(onScan = { scanner.launch(scanOptions()) }, scanned = scanned, onScanHandled = { scanned = null }) }
    }

    override fun onResume() {
        super.onResume()
        // Ask now rather than waiting for the poll: somebody opening the app is
        // the one moment they are definitely waiting for an answer.
        WorkManager.getInstance(this).enqueueUniqueWork(
            SyncWorker.UNIQUE + "-now",
            ExistingWorkPolicy.KEEP,
            OneTimeWorkRequestBuilder<SyncWorker>().build()
        )
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
