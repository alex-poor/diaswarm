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
            Result.success()
        } catch (e: Throwable) {
            // An unreachable peer is the ordinary condition of a swarm, not an
            // error. The age on screen is what tells somebody it has gone on
            // too long.
            Log.w(TAG, "sync pass failed", e)
            Result.success()
        }
    }

    companion object {
        const val TAG = "diaswarm"
        const val UNIQUE = "diaswarm-sync"
        const val EVERY_SECONDS = 120L
    }
}
