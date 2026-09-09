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
}
