package app.aaps.plugins.sync.swarm

import app.aaps.core.keys.interfaces.Preferences
import app.aaps.plugins.sync.swarm.keys.SwarmStringKey

/**
 * The queue of readers waiting to be withdrawn, behind the single
 * [SwarmStringKey.RevokeReader] string preference.
 *
 * **A WITHDRAWAL MUST NOT BE LOST, EVER — IT IS ACCESS CONTROL.** Stopping
 * several readers taps the Stop button several times before a sync pass runs,
 * and each tap used to `put` the key straight into the pref, so all but the last
 * were overwritten: the rows left the screen, but only one reader was ever
 * removed from the group and the rest kept reading. Observed live 2026-10-28,
 * "I removed everyone and phone B still gets my data".
 *
 * So the pref holds a newline-separated set, and append and drain are
 * serialised on this object's monitor so a tap landing during a pass cannot be
 * dropped: anything added after a drain claims simply waits for the next pass.
 */
internal object SwarmRevokeQueue {

    /**
     * Sentinel meaning "withdraw everyone", queued like a reader. Not a valid
     * reader key (those are 64 lowercase hex), so it can never collide with a
     * pasted one. Drained like any other entry; the sync pass turns it into a
     * [SwarmNative.keysRevokeAll] off the group's live members, which works even
     * when the readers book has been emptied.
     */
    const val ALL = "__all__"

    @Synchronized
    fun add(preferences: Preferences, who: String) {
        val wanted = who.trim()
        if (wanted.isEmpty()) return
        val set = split(preferences.get(SwarmStringKey.RevokeReader)).toMutableSet()
        set.add(wanted)
        preferences.put(SwarmStringKey.RevokeReader, set.joinToString("\n"))
    }

    /** Claim every queued withdrawal and empty the queue, atomically. */
    @Synchronized
    fun drain(preferences: Preferences): List<String> {
        val all = split(preferences.get(SwarmStringKey.RevokeReader))
        if (all.isNotEmpty()) preferences.put(SwarmStringKey.RevokeReader, "")
        return all
    }

    private fun split(raw: String): List<String> =
        raw.lines().map { it.trim() }.filter { it.isNotEmpty() }.distinct()
}
