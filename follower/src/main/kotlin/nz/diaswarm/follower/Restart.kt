package nz.diaswarm.follower

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.util.Log

/**
 * Put the stay-awake service back after a reboot or an app update.
 *
 * **BECAUSE IT DID NOT COME BACK, AND NOTHING SAID SO.** `StayAwake` is started
 * from `MainActivity.onCreate`, so anything that kills the process without the
 * user reopening the app leaves the setting switched on and doing nothing: an
 * update, a force-stop, a reboot. It was found during a test that would have
 * measured nothing — the service had been dead since the previous install.
 *
 * That is the worst shape a setting can have. "On" has to mean on.
 *
 * `START_STICKY` covers Android killing the service for memory; it does not
 * cover the process never being started at all, which is what these two
 * broadcasts are for.
 */
class Restart : BroadcastReceiver() {

    override fun onReceive(context: Context, intent: Intent) {
        when (intent.action) {
            Intent.ACTION_BOOT_COMPLETED,
            Intent.ACTION_MY_PACKAGE_REPLACED -> {
                Log.i(SyncWorker.TAG, "restart after ${intent.action} — reapplying stay-awake")
                StayAwake.apply(context)
            }
        }
    }
}
