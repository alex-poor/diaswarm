package nz.diaswarm.follower

import android.content.Context
import android.util.Log
import nz.diaswarm.jni.SwarmNative
import java.io.File
import java.util.TimeZone

/**
 * This follower's `diaswarm-keys` identity, and the logs it carries.
 *
 * **A FOLLOWER NEEDS A KEYS VAULT ONLY TO BE GRANTED.** It seals nothing. What
 * it has to own is an identity: a grant is agreed against a key bundle, and the
 * subject can only grant one it has been given. That is what goes in the
 * invite, and it is the whole reason this exists on the reading side.
 *
 * **ONE IDENTITY, SEVERAL VAULTS.** Following three people means three joined
 * vaults, and all three must join with the key manager *this* vault holds —
 * the one each subject granted against. A joined vault that minted its own
 * would be a different member to the one that was granted and would read
 * nothing, with no error to say so.
 */
object SwarmKeys {

    private const val TAG = SyncWorker.TAG

    val offsetMs: Long get() = TimeZone.getDefault().rawOffset.toLong()

    /** Where this follower's own keys vault and its store live. */
    fun dir(context: Context): File = File(SwarmPaths.base(context), "keys").also { it.mkdirs() }

    /** Where a subject's joined vault lives: one directory per subject. */
    fun joinedDir(context: Context, subject: String): File =
        File(dir(context), "joined/$subject").also { it.mkdirs() }

    /**
     * Open this follower's own keys vault. 0 if the preference is off or the
     * pool is not up.
     *
     * The caller closes it. Handles are per-pass: the pool owns the store and
     * the runtime, so a handle is cheap and must not outlive the work it was
     * opened for.
     */
    fun open(context: Context): Long {
        if (!Prefs.keysVault(context)) return 0L
        val pool = Endpoint.handle
        if (pool == 0L) return 0L
        return try {
            SwarmNative.keysOpen(
                pool,
                dir(context).absolutePath,
                SwarmPaths.identity(context).absolutePath,
                offsetMs
            )
        } catch (e: Throwable) {
            Log.w(TAG, "keys vault would not open: $e")
            0L
        }
    }

    /**
     * This follower's identity for an invite, or empty.
     *
     * Empty means a v2 invite, which every build already installed can read.
     * See [Prefs.keysVault] for why that is the default.
     */
    fun identity(context: Context): String {
        val handle = open(context)
        if (handle == 0L) return ""
        return try {
            SwarmNative.keysIdentity(handle)
        } catch (e: Throwable) {
            Log.w(TAG, "keys identity threw: $e")
            ""
        } finally {
            SwarmNative.keysClose(handle)
        }
    }
}
