package app.aaps.plugins.sync.swarm

import android.content.Context
import app.aaps.core.keys.interfaces.Preferences
import app.aaps.plugins.sync.swarm.keys.SwarmBooleanKey
import nz.diaswarm.jni.SwarmNative
import java.io.File
import java.util.TimeZone

/**
 * The keys vault, as the rest of the plugin needs to see it.
 *
 * **ONE PLACE, BECAUSE TWO CALLERS WANT THE SAME ANSWER.** The sync selector
 * seals into this vault and the plugin puts its identity in an invite, and they
 * must agree about whether there is one at all — a phone that advertises a keys
 * identity it never seals into hands followers a v3 invite, makes them join a
 * group, and gives them nothing to read. Getting that wrong in one of two
 * copies is exactly the kind of drift this object exists to prevent.
 */
object SwarmKeys {

    /** The epoch offset, as [DataSyncSelectorSwarmImpl] derives it. */
    val offsetMs: Long get() = TimeZone.getDefault().rawOffset.toLong()

    /** Where the keys vault lives. Also where its SQLite store is. */
    fun dir(context: Context): File = File(SwarmPaths.base(context), "keys").also { it.mkdirs() }

    /**
     * This phone's keys identity for an invite, or empty.
     *
     * **EMPTY UNLESS THE KEYS VAULT IS ACTUALLY BEING WRITTEN**, and empty
     * means a v2 invite — which every build already installed can read. Only a
     * phone that can genuinely grant on the keys vault emits something older
     * builds refuse, and they then say so clearly rather than half-working.
     */
    fun identity(context: Context, preferences: Preferences): String {
        if (!preferences.get(SwarmBooleanKey.ShadowSpacesVault)) return ""
        val pool = SwarmEndpoint.handle
        if (pool == 0L) return ""
        return try {
            val handle = SwarmNative.keysOpen(
                pool,
                dir(context).absolutePath,
                SwarmPaths.identity(context).absolutePath,
                offsetMs
            )
            if (handle == 0L) return ""
            try {
                SwarmNative.keysIdentity(handle)
            } finally {
                SwarmNative.keysClose(handle)
            }
        } catch (e: Throwable) {
            // CAUGHT, INCLUDING ERRORS. Showing an invite must not be able to
            // take the app down: a stale .so throws UnsatisfiedLinkError here,
            // and the honest answer is "no keys identity", not a crash.
            ""
        }
    }
}
