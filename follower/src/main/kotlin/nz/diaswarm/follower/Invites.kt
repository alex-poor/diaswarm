package nz.diaswarm.follower

import android.content.Context

/**
 * An invite somebody gave us, waiting for the next sync pass to act on it.
 *
 * A QUEUE RATHER THAN A DIRECT CALL, because following touches the same store
 * the sync pass is reading and a scan happens on the UI thread. One code path
 * changes what this phone follows, and it is not the one with a camera open.
 */
object Invites {

    private const val FILE = "diaswarm-follower"
    private const val PENDING = "pending_invite"

    fun queue(context: Context, invite: String) =
        context.getSharedPreferences(FILE, Context.MODE_PRIVATE)
            .edit().putString(PENDING, invite.trim()).apply()

    /** Read it and clear it, so a bad invite is not retried for ever in silence. */
    fun takePending(context: Context): String {
        val p = context.getSharedPreferences(FILE, Context.MODE_PRIVATE)
        val v = p.getString(PENDING, "").orEmpty()
        if (v.isNotEmpty()) p.edit().remove(PENDING).apply()
        return v
    }
}
