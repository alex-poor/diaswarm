package nz.diaswarm.follower

import android.content.Context
import android.util.Log
import androidx.work.Worker
import androidx.work.WorkerParameters
import nz.diaswarm.jni.SwarmNative

/**
 * Ask the people this phone follows whether anything has changed.
 *
 * **THE CADENCE IS THE DATA'S, NOT A PREFERENCE.** A CGM produces a reading
 * every one to five minutes, so polling faster mostly discovers that nothing has
 * changed — which still costs a connection. Two minutes sits under the data's
 * own rhythm without polling into the gaps.
 *
 * ANDROID DECIDES THE REST. Doze batches background work, so "two minutes" means
 * two minutes while the phone is awake and something longer in a pocket. That is
 * the platform, not the design, and it is exactly why every reading on screen
 * carries its age rather than appearing as a bare number that implies it is
 * current.
 */
class SyncWorker(context: Context, params: WorkerParameters) : Worker(context, params) {

    override fun doWork(): Result {
        return try {
            SwarmNative.check()
            val store = SwarmPaths.store(applicationContext).absolutePath

            // Act on anything the user asked for before fetching, so a freshly
            // pasted invite is followed in this pass rather than the next one.
            Prefs.let { }
            val pending = Invites.takePending(applicationContext)
            if (pending.isNotEmpty()) {
                val n = SwarmNative.netFollow(store, pending)
                Log.i(TAG, "followed an invite: $n")
            }

            if (Follower.following(applicationContext).isEmpty()) {
                Log.i(TAG, "following nobody — nothing to fetch")
                return Result.success()
            }
            val reached = SwarmNative.netRefresh(Endpoint.handle, store)
            Log.i(TAG, "refreshed $reached subject(s)")

            // **CARRY THE KEYS LOGS TOO, AND SAY HOW MANY.** Without this the
            // replicator subscribes to nothing and every keys read finds an
            // empty store, which looks exactly like "not granted yet". Logged
            // either way: a pass that carried nothing and a pass that carried
            // everything must not read the same.
            if (Prefs.keysVault(applicationContext) && Endpoint.handle != 0L) {
                val carried = SwarmNative.keysCarryAll(
                    Endpoint.handle,
                    store,
                    SwarmPaths.identity(applicationContext).absolutePath
                )
                Log.i(TAG, if (carried < 0) "keys carry unavailable ($carried)" else "keys carrying $carried log(s)")
                if (carried > 0) verifyControlLogs(applicationContext)

                // **OFFER OUR KEYS IDENTITY TO PEOPLE WHO ALREADY GRANTED US.**
                // They granted this phone on the old vault, possibly months
                // ago; the new one needs an identity they have never seen, and
                // only we can supply it with a proof only the two of us can
                // make. Nobody scans anything. A subject that has not moved to
                // the keys vault, or is too old to understand the request,
                // answers 0 — and that is the status quo, not a failure.
                val identity = SwarmKeys.identity(applicationContext)
                if (identity.isNotEmpty()) {
                    for (subject in Follower.following(applicationContext)) {
                        // Only where we have something to hand over to: a
                        // subject with no keys identity of their own has no
                        // keys vault to be granted on.
                        if (subject.keys.isEmpty()) continue
                        // AND NOT EVERY PASS. See [Prefs.handedOverAt]: this
                        // was asking once a minute, for ever, and the subject
                        // was granting once a minute, for ever.
                        val since = System.currentTimeMillis() -
                            Prefs.handedOverAt(applicationContext, subject.key)
                        if (since < HANDOVER_RETRY_MS) continue
                        val took = SwarmNative.netHandOver(
                            Endpoint.handle,
                            store,
                            SwarmPaths.identity(applicationContext).absolutePath,
                            subject.key,
                            identity
                        )
                        if (took == 1L) {
                            Prefs.setHandedOverAt(applicationContext, subject.key, System.currentTimeMillis())
                            Log.i(TAG, "handed over to ${subject.short}")
                        }
                    }
                }
            }
            Result.success()
        } catch (e: Throwable) {
            // An unreachable peer is the ordinary condition of a swarm, not an
            // error. The age on screen is what tells somebody it has gone on
            // too long.
            Log.w(TAG, "sync pass failed", e)
            Result.success()
        }
    }

    /**
     * Check each followed subject's control log still holds together.
     *
     * **THE POINT OF A TAMPER-EVIDENT LOG IS SOMEBODY CHECKING IT.** D13's
     * grant log replicates precisely so that truncating or altering it is
     * detectable, and §11 offers that *in place of* a read log — but the code
     * that detects it had never run outside a test on either phone. A property
     * nobody evaluates is a claim, not a property.
     *
     * Here rather than on the subject's phone because the subject cannot catch
     * themselves: this runs on the copy this follower was actually served.
     *
     * Once per subject per pass, on a log that holds one entry per grant and
     * revocation — a handful of hashes, not a thing to be clever about. Logged
     * quietly when intact and loudly when not, and it changes nothing else:
     * a broken chain is something a person has to look at, not something an
     * app should act on by itself.
     */
    private fun verifyControlLogs(context: Context) {
        val handle = SwarmKeys.open(context)
        if (handle == 0L) return
        try {
            for (subject in Follower.following(context)) {
                if (subject.keys.isEmpty()) continue
                val verdict = try {
                    SwarmNative.keysVerifyControl(handle, subject.keys)
                } catch (e: Throwable) {
                    Log.w(TAG, "chain check threw for ${subject.short}: $e")
                    continue
                }
                when {
                    verdict.startsWith("ok ") ->
                        Log.i(TAG, "grant log intact for ${subject.short}: $verdict")
                    verdict.startsWith("broken") ->
                        Log.e(TAG, "GRANT LOG BROKEN for ${subject.short}: $verdict")
                    // NO SILENT BRANCH. This used to swallow an empty answer
                    // as "nothing replicated yet", and an empty answer was
                    // exactly what a wrong argument produced — so the check ran
                    // every pass, checked nothing, and said nothing.
                    else -> Log.w(TAG, "chain check for ${subject.short} said: '$verdict'")
                }
            }
        } finally {
            SwarmNative.keysClose(handle)
        }
    }

    companion object {
        const val TAG = "diaswarm"

        /** How long to leave a subject alone between keys-identity offers. */
        private const val HANDOVER_RETRY_MS = 30 * 60 * 1000L
        const val UNIQUE = "diaswarm-sync"

        /**
         * How often to ask while somebody is looking at the screen.
         *
         * A CGM produces a reading every one to five minutes, so asking faster
         * mostly discovers that nothing changed — and still costs a connection.
         */
        const val EVERY_SECONDS = 120L
    }
}

/** The one place that asks for a pass, so there is one policy and not three. */
object Sync {

    private const val NOW = "diaswarm-sync-now"

    /**
     * Ask for a pass now.
     *
     * `KEEP`, so a pass already running is left to finish: it is about to do
     * this work anyway, and cancelling it mid-fetch to start again is how a
     * refresh loop becomes a refresh stall.
     */
    fun now(context: android.content.Context) {
        androidx.work.WorkManager.getInstance(context).enqueueUniqueWork(
            NOW,
            androidx.work.ExistingWorkPolicy.KEEP,
            androidx.work.OneTimeWorkRequestBuilder<SyncWorker>().build()
        )
    }
}
