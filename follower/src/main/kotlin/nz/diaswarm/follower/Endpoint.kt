package nz.diaswarm.follower

import android.content.Context
import android.util.Log
import nz.diaswarm.jni.SwarmNative

/**
 * The running network endpoint, and the window that lets one scan do both jobs.
 *
 * Held as a process-wide handle because it is a live QUIC endpoint with a relay
 * connection behind it: opening a second one would put two peers with the same
 * identity in the pool, announcing against each other.
 */
object Endpoint {

    @Volatile
    var handle: Long = 0L
        private set

    /**
     * How long after showing your code this phone accepts an invite pushed back
     * at it.
     *
     * Long enough for somebody to pick up a phone and line up a camera; short
     * enough that a code shown once in a cafe is not an open door all
     * afternoon. Outside it, a pushed invite is refused — otherwise anyone who
     * knows this node id could make the phone carry their ciphertext, and node
     * ids are announced in the pool.
     */
    const val OFFER_WINDOW_SECONDS = 180L

    @Synchronized
    fun start(context: Context) {
        if (handle != 0L) return
        SwarmNative.check()
        // BEFORE THE ENDPOINT, because the network stack reaches for it as soon
        // as it starts watching for connectivity changes.
        SwarmNative.initAndroid(context.applicationContext)
        val h = SwarmNative.swarmJoin(
            SwarmPaths.store(context).absolutePath,
            SwarmPaths.nodeKey(context).absolutePath,
            // **NOTHING WHEN THE FEATURE IS OFF, AND THAT IS NOT AN
            // OPTIMISATION.** This used to be passed unconditionally, so the
            // pool built a KeysReplicator whatever the preference said. Once
            // keysCarryAll had run — while the toggle was on — its log-sync
            // subscriptions stayed live, because turning a preference off does
            // not unsubscribe anything. p2panda sync then ran alongside the
            // core vault's own polling on the same endpoint and the readings a
            // follower showed went from seconds old to minutes old.
            //
            // The person watching a graph turned the feature off and it kept
            // costing them. A switch that only takes effect on the next launch
            // is not a switch.
            if (Prefs.keysVault(context)) SwarmKeys.dir(context).absolutePath else ""
        )
        if (h == 0L) {
            Log.w(SyncWorker.TAG, "could not join the pool")
            return
        }
        handle = h
        Log.i(SyncWorker.TAG, "in the pool as ${SwarmNative.swarmNodeId(h).take(16)}")
    }

    /** This phone's own invite, or empty until it is serving. */
    fun invite(context: Context): String {
        if (handle == 0L) return ""
        SwarmNative.check()
        val subject = SwarmNative.vaultSubject(SwarmPaths.identity(context).absolutePath)
        val node = SwarmNative.swarmNodeId(handle)
        if (subject.isEmpty() || node.isEmpty()) return ""
        return SwarmNative.inviteFor(subject, node, Follower.PURPOSE, SwarmKeys.identity(context))
    }

    /** Showing the code is the consent: accept a pushed invite from now. */
    fun expectOffer() {
        if (handle != 0L) SwarmNative.swarmExpectOffer(handle, OFFER_WINDOW_SECONDS)
    }
}
