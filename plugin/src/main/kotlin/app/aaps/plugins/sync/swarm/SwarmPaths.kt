package app.aaps.plugins.sync.swarm

import android.content.Context
import java.io.File

/**
 * Where things live on the device.
 *
 * A STORE, NOT A VAULT. A node that serves only its own vault is a personal
 * server — reachable exactly when its owner's phone is awake. A node that
 * serves the vaults it has replicated is a peer, and a reader can get a
 * subject's history from anyone holding it. That is the difference between a
 * set of phones and a swarm (feasibility.md §9.2), and it is a directory
 * layout, not an algorithm.
 *
 * ```text
 * files/diaswarm/
 *   subject.id            this device's keys — never leaves
 *   node.key              this device's endpoint identity
 *   store/<subject>/      one vault per subject, named by public key so two
 *                         peers replicating the same person agree on the path
 * ```
 */
object SwarmPaths {

    fun base(context: Context): File = File(context.filesDir, "diaswarm").also { it.mkdirs() }

    fun identity(context: Context): File = File(base(context), "subject.id")

    fun nodeKey(context: Context): File = File(base(context), "node.key")

    fun store(context: Context): File = File(base(context), "store").also { it.mkdirs() }

    /**
     * This device's own vault, inside the store.
     *
     * Migrates a vault written before the store existed, once. Moved rather
     * than copied: two vaults for one subject drifting apart is worse than
     * either of them, and the sealing side would keep writing to whichever it
     * was handed.
     */
    fun vault(context: Context, @Suppress("UNUSED_PARAMETER") caller: Class<*>): File {
        val subject = SwarmNative.vaultSubject(identity(context).absolutePath)
        val dest = File(store(context), subject)
        val legacy = File(base(context), "vault")
        if (subject.isNotEmpty() && !File(dest, "meta.json").exists() &&
            File(legacy, "meta.json").exists()
        ) {
            legacy.renameTo(dest)
        }
        dest.mkdirs()
        return dest
    }
}
