package nz.diaswarm.follower

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Context
import android.content.Intent
import android.os.Build
import android.os.IBinder
import android.util.Log

/**
 * Keeps this phone fetching while the screen is off, and says so on the
 * notification shade for as long as it does.
 *
 * **THE PHONE NAMED ITS OWN REMEDY.** Two hours unplugged and still, with a
 * `WifiLock` held throughout, produced forty minutes of deep doze with no relay
 * connection and no fetches. `dumpsys netpolicy` said why, exactly:
 *
 * ```
 * UID=10253 blocked_state={effective=DOZE|APP_BACKGROUND}
 * ```
 *
 * Two flags, two remedies, and neither of them is a lock on the radio — the
 * radio was up the whole time. `APP_BACKGROUND` is cleared by running a
 * foreground service; `DOZE` is cleared by the battery-optimisation whitelist,
 * which only the user can grant. This class is the first half. [StayAwake.ask]
 * is the second.
 *
 * **AND IT DOES THE WORK ITSELF, rather than leaning on WorkManager.** A
 * periodic worker is exactly what Doze defers; keeping a service alive so that
 * a deferred worker can run later would be theatre. The loop is here, on the
 * same two-minute cadence the foreground app uses.
 *
 * The notification is not a nuisance to be minimised. It is the honest price of
 * this: an app that keeps a network connection open while you are asleep should
 * be visible in the shade, and stopping it is one tap from there.
 */
class StayAwake : Service() {

    @Volatile
    private var running = false

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        if (intent?.action == STOP) {
            stopSelf()
            return START_NOT_STICKY
        }
        startForeground(NOTIFICATION_ID, notification())
        if (!running) {
            running = true
            Thread({ loop() }, "diaswarm-stayawake").start()
        }
        // STICKY: if Android kills this for memory, the user asked for it to be
        // running and should get it back without opening the app.
        return START_STICKY
    }

    override fun onDestroy() {
        running = false
        Log.i(SyncWorker.TAG, "stay-awake service stopped")
        super.onDestroy()
    }

    private fun loop() {
        Log.i(SyncWorker.TAG, "stay-awake service started")
        while (running) {
            try {
                Endpoint.start(applicationContext)
                Sync.now(applicationContext)
            } catch (e: Throwable) {
                Log.w(SyncWorker.TAG, "stay-awake pass failed: $e")
            }
            var slept = 0L
            while (running && slept < SyncWorker.EVERY_SECONDS * 1000) {
                Thread.sleep(500)
                slept += 500
            }
        }
    }

    private fun notification(): Notification {
        val manager = getSystemService(NotificationManager::class.java)
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            manager.createNotificationChannel(
                NotificationChannel(CHANNEL, "Keeping up to date", NotificationManager.IMPORTANCE_LOW)
                    .also { it.description = "Shown while ayni is fetching with the screen off." }
            )
        }
        val open = PendingIntent.getActivity(
            this,
            0,
            Intent(this, MainActivity::class.java),
            PendingIntent.FLAG_IMMUTABLE
        )
        return Notification.Builder(this, CHANNEL)
            .setContentTitle("Keeping up to date")
            .setContentText("Fetching readings while the screen is off")
            .setSmallIcon(android.R.drawable.stat_notify_sync)
            .setContentIntent(open)
            .setOngoing(true)
            .build()
    }

    companion object {
        private const val CHANNEL = "diaswarm-stay-awake"
        private const val NOTIFICATION_ID = 4712
        private const val STOP = "nz.diaswarm.follower.STOP_STAY_AWAKE"

        /**
         * Is this app exempt from doze — the other half, which only the user
         * can grant?
         *
         * The service clears `APP_BACKGROUND`. Nothing an app can do clears
         * `DOZE`; it needs the battery-optimisation exemption, and Android
         * deliberately makes that a decision a person makes rather than a
         * permission an app takes.
         */
        fun exemptFromDoze(context: Context): Boolean =
            context.getSystemService(android.os.PowerManager::class.java)
                ?.isIgnoringBatteryOptimizations(context.packageName) ?: false

        /**
         * Open the settings screen where that exemption is granted.
         *
         * **THE PLAIN LIST, NOT THE DIRECT PROMPT.** There is an intent that
         * asks for this in one tap — `REQUEST_IGNORE_BATTERY_OPTIMIZATIONS` —
         * and it is exactly the one F-Droid and Play treat as a red flag,
         * because it is the one apps abuse. Sending somebody to the list they
         * can already reach themselves is slower by one tap and asks for
         * nothing it has not explained first.
         */
        fun ask(context: Context) {
            val intent = Intent(android.provider.Settings.ACTION_IGNORE_BATTERY_OPTIMIZATION_SETTINGS)
                .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
            try {
                context.startActivity(intent)
            } catch (e: Throwable) {
                Log.w(SyncWorker.TAG, "no battery optimisation screen: $e")
            }
        }

        /** Start or stop the service to match the preference. Safe to call often. */
        fun apply(context: Context) {
            val want = Prefs.stayReachable(context)
            val intent = Intent(context, StayAwake::class.java)
            try {
                if (want) {
                    context.startForegroundService(intent)
                } else {
                    context.stopService(intent)
                }
            } catch (e: Throwable) {
                // A background start can be refused. The preference stays on and
                // the next time the app is opened this runs again.
                Log.w(SyncWorker.TAG, "could not ${if (want) "start" else "stop"} stay-awake: $e")
            }
        }
    }
}
