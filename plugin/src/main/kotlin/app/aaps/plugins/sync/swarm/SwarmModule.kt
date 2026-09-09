package app.aaps.plugins.sync.swarm

import app.aaps.plugins.sync.swarm.workers.SwarmDataSyncWorker
import dagger.Module
import dagger.android.ContributesAndroidInjector

/**
 * Dagger wiring for the swarm add-on.
 *
 * Deliberately tiny, and deliberately not binding `DataSyncSelectorSwarmImpl`
 * to any interface AAPS already knows about. The selector is reached through
 * this plugin alone, so nothing in the app can acquire it by asking for a
 * general `DataSyncSelector` and get a surprise.
 */
@Module
abstract class SwarmModule {

    /**
     * The worker needs this: `LoggingWorker` injects itself out of the
     * application's injector in its constructor, so a worker with no
     * contribution here fails at construction rather than at compile time.
     */
    @ContributesAndroidInjector
    abstract fun contributesSwarmDataSyncWorker(): SwarmDataSyncWorker

    /**
     * The scan screen needs this for the same reason, and the cost of leaving
     * it out is worse than a compile error.
     *
     * A `DaggerAppCompatActivity` with no contribution here throws
     * "No injector factory bound" the moment it is launched — which, because
     * it happens in `onCreate`, takes the whole app down. Observed exactly
     * once, on a phone that was running a closed loop: tapping a settings row
     * killed AAPS and it restarted. Nothing about it fails at build time.
     */
    @ContributesAndroidInjector
    abstract fun contributesSwarmScanActivity(): SwarmScanActivity
}
