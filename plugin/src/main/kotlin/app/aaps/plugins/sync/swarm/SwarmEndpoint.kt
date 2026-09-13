package app.aaps.plugins.sync.swarm

/**
 * The one running endpoint, shared between the plugin that starts it and the
 * sync pass that fetches through it.
 *
 * WHY THIS EXISTS RATHER THAN AN INJECTED DEPENDENCY. [SwarmPlugin] owns the
 * endpoint's lifetime — it starts it when enabled and stops it when not — while
 * `DataSyncSelectorSwarmImpl` is built separately by Dagger and needs the same
 * handle. Threading a mutable, nullable native pointer through the graph would
 * be more machinery than the thing it carries.
 *
 * **A peer has one identity, and this is what enforces it.** Peers remember
 * whoever fetched from them and pass that address on to later followers, so a
 * fetch made from a throwaway endpoint hands out somewhere that stops existing
 * the moment the sync ends. Serving and fetching through the same endpoint is
 * what makes discovery mean anything.
 *
 * 0 means not serving. That is a legitimate state, not an error: fetching still
 * works, this phone simply will not be remembered as somewhere to look.
 */
object SwarmEndpoint {

    @Volatile
    var handle: Long = 0L

    /**
     * Ask the owner to rebuild the pool if the keys preference has moved.
     *
     * **BECAUSE THE POOL DECIDES ONCE, AT JOIN.** `swarmJoin` either builds the
     * keys replicator or does not, from the directory it is handed, and no
     * later call can add one — so flipping the preference on used to change
     * nothing until the process was restarted, while every keys call returned
     * -3 and the logs blamed the vault.
     *
     * Set by [SwarmPlugin] while it is serving, and called from the sync pass,
     * which is the only thing here that runs on a cadence. Null when nothing is
     * serving, which is the honest answer rather than a no-op: there is no pool
     * to disagree with.
     */
    @Volatile
    var rejoinIfPreferencesChanged: (() -> Unit)? = null
}
