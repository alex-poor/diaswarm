package nz.diaswarm.follower

import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.os.Build
import android.util.Log
import androidx.compose.ui.graphics.toArgb

/**
 * Telling somebody when the person they follow goes low or high — the reason to
 * follow at all.
 *
 * **IT ONLY EVER FIRES ON A FRESH READING, AND THAT IS THE WHOLE DISCIPLINE
 * HERE.** §12.3 is that a follower's dangerous failure is a number that looks
 * current and is hours old. An alert is that failure with a sound attached: a
 * phone that buzzes "urgent low" about a reading from three hours ago is worse
 * than one that stays silent, because it spends the one thing a carer has —
 * their trust that a buzz means now. So a reading older than
 * [Glucose.STALE_AFTER_MS] raises nothing, ever. The age on screen is for
 * looking; an alert is for acting, and you cannot act on the past.
 *
 * **IT RUNS IN THE SYNC PASS, NOT ON THE SCREEN.** [SyncWorker] is the one place
 * a reading newly arrives, whether the app is open or the stay-awake service is
 * carrying it in the background — so the check lives there and an alert does not
 * depend on anyone watching.
 *
 * It does not compute anything clinical. It compares a delivered number to the
 * same lines the chart colours against and to two fixed thresholds, and it never
 * invents a value, a prediction or a trend.
 */
object Alerts {

    private const val CHANNEL = "diaswarm-alerts"

    /**
     * Once in a level, stay quiet until it either gets worse or this long has
     * passed. A CGM at 3.0 produces a reading a minute; buzzing every one of
     * them is how an alert becomes noise to be silenced, which is the same as no
     * alert at all. Twenty minutes still being low is worth saying again.
     */
    private const val REARM_MS = 20L * 60_000L

    /**
     * The window read to find the newest reading. Short on purpose: this is "what
     * is true now", not a history, and reading an hour keeps the native call
     * cheap while still spanning any plausible gap between passes.
     */
    private const val LOOK_BACK_HOURS = 1

    /**
     * A reading's band, in severity. Low and high are separate directions; the
     * [rank] is how far from ordinary, so an escalation within a direction
     * (low → urgent) can re-alert while an improvement (urgent → low) does not.
     */
    enum class Level(val rank: Int, val label: String) {
        URGENT_LOW(3, "urgent low"),
        LOW(1, "low"),
        HIGH(1, "high"),
        VERY_HIGH(3, "very high");
    }

    private fun key(level: Level) = "alert_on_${level.name.lowercase()}"

    /**
     * Whether each level alerts, and the defaults.
     *
     * **HIGH IS OFF, THE OTHER THREE ON, AND THAT IS A JUDGEMENT ABOUT WHO IS
     * HOLDING THE PHONE.** A follower is usually not the person who can correct
     * a drift to 11 — they are someone who wants to know about the two ends that
     * are an emergency and the slide toward one. A reading over the high line is
     * ordinary diabetes; a reading over 13.9, or any low, is the thing worth a
     * sound. Anyone who does want every high turns it on, once.
     */
    private fun defaultOn(level: Level) = level != Level.HIGH

    fun enabled(c: Context, level: Level): Boolean =
        prefs(c).getBoolean(key(level), defaultOn(level))

    fun setEnabled(c: Context, level: Level, on: Boolean) =
        prefs(c).edit().putBoolean(key(level), on).apply()

    fun anyEnabled(c: Context): Boolean = Level.entries.any { enabled(c, it) }

    private fun prefs(c: Context) =
        c.getSharedPreferences("diaswarm-follower", Context.MODE_PRIVATE)

    /**
     * Where a reading sits, most severe first, or null when it is nothing to say.
     *
     * The urgent-low and very-high lines are fixed clinical thresholds, checked
     * first so they hold even when the viewer has turned colouring off — see
     * [Glucose]. Low and high need the viewer's own lines, so with the band off
     * (lines equal) they cannot be classified and the answer is null.
     */
    fun bandFor(mgdl: Double, lowLine: Double, highLine: Double): Level? {
        val bandOff = highLine - lowLine < 0.5
        return when {
            mgdl <= Glucose.URGENT_LOW_MGDL -> Level.URGENT_LOW
            mgdl >= Glucose.VERY_HIGH_MGDL -> Level.VERY_HIGH
            bandOff -> null
            mgdl < lowLine -> Level.LOW
            mgdl > highLine -> Level.HIGH
            else -> null
        }
    }

    /**
     * Should a reading in [band] raise anything, given what is enabled.
     *
     * **A VERY-HIGH ALSO SATISFIES HIGH, AND AN URGENT-LOW ALSO SATISFIES LOW.**
     * Somebody who asked to be told about lows and left the extreme off still
     * means "and obviously the worse one"; the dangerous miss would be a 2.8 that
     * stays silent because only "low" was ticked. So the extreme fires on either
     * its own switch or the milder one in the same direction.
     */
    private fun shouldFire(c: Context, band: Level): Boolean = when (band) {
        Level.URGENT_LOW -> enabled(c, Level.URGENT_LOW) || enabled(c, Level.LOW)
        Level.LOW -> enabled(c, Level.LOW)
        Level.HIGH -> enabled(c, Level.HIGH)
        Level.VERY_HIGH -> enabled(c, Level.VERY_HIGH) || enabled(c, Level.HIGH)
    }

    /**
     * Look at the newest reading for each person followed and raise what is due.
     *
     * Called from the sync pass once the vault has been refreshed, so it runs on
     * exactly the readings that just arrived, open or asleep.
     */
    fun check(context: Context) {
        if (!anyEnabled(context)) return
        // Nothing to post with: on 33+ the permission is a runtime grant, and
        // without it a posted notification is silently dropped. Asking is the
        // app's job (MainActivity); here we simply do not pretend.
        if (!canNotify(context)) return

        val lowLine = Prefs.lowLine(context)
        val highLine = Prefs.highLine(context)

        for (subject in Follower.following(context).filter { it.reached }) {
            val newest = Follower.readings(context, subject, LOOK_BACK_HOURS)
                .maxByOrNull { it.at }

            // No reading, or an old one, clears nothing and raises nothing. The
            // stored state is left as it is: if they were low an hour ago and
            // the feed went quiet, re-arming now and firing the moment a fresh
            // low arrives is exactly right, and firing on the stale one is the
            // §12.3 failure this whole object is built around.
            if (newest == null || Glucose.isStale(newest.at)) continue

            val band = bandFor(newest.mgdl, lowLine, highLine)
            val lastRank = Prefs.alertLevel(context, subject.key)

            // Back in range: forget the excursion, so the next one alerts afresh.
            if (band == null || !shouldFire(context, band)) {
                if (lastRank >= 0) Prefs.clearAlert(context, subject.key)
                continue
            }

            val lastAt = Prefs.alertAt(context, subject.key)
            val worsened = band.rank > lastRank
            val reArmed = System.currentTimeMillis() - lastAt > REARM_MS
            // First time in a level, a worse level, or long enough since the
            // last buzz that it is worth repeating.
            if (lastRank < 0 || worsened || reArmed) {
                post(context, subject, band, newest)
                Prefs.setAlert(context, subject.key, band.rank)
            }
        }
    }

    /** On 33+ the notification permission is a runtime grant; below it, always true. */
    private fun canNotify(context: Context): Boolean {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.TIRAMISU) return true
        return context.checkSelfPermission(android.Manifest.permission.POST_NOTIFICATIONS) ==
            android.content.pm.PackageManager.PERMISSION_GRANTED
    }

    private fun post(context: Context, subject: Follower.Subject, band: Level, reading: Follower.Reading) {
        val manager = context.getSystemService(NotificationManager::class.java) ?: return
        ensureChannel(manager)

        val value = "${Prefs.show(context, reading.mgdl)} ${Prefs.unitLabel(context)}"
        val tone = Glucose.color(reading.mgdl, Prefs.lowLine(context), Prefs.highLine(context), stale = false)

        val open = PendingIntent.getActivity(
            context, 0,
            Intent(context, MainActivity::class.java),
            PendingIntent.FLAG_IMMUTABLE
        )

        val notification = android.app.Notification.Builder(context, CHANNEL)
            .setContentTitle("${subject.label} — ${band.label}")
            .setContentText("$value · ${Follower.ageWords(reading.at)}")
            .setSmallIcon(android.R.drawable.stat_notify_error)
            .setColor(tone.toArgb())
            .setContentIntent(open)
            .setCategory(android.app.Notification.CATEGORY_ALARM)
            .setAutoCancel(true)
            .build()

        // One id per subject, so a new alert for the same person replaces the
        // old rather than stacking a column of them.
        manager.notify(subject.key.hashCode(), notification)
        Log.i(SyncWorker.TAG, "alert: ${subject.short} ${band.label} at $value")
    }

    private fun ensureChannel(manager: NotificationManager) {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.O) return
        // HIGH, not the stay-awake channel's LOW: this one is meant to be heard.
        // It sits apart so a person can silence "keeping up to date" and keep
        // the alerts, or the other way round, from the system settings.
        manager.createNotificationChannel(
            NotificationChannel(CHANNEL, "Glucose alerts", NotificationManager.IMPORTANCE_HIGH)
                .also { it.description = "When the person you follow goes low or high." }
        )
    }
}
