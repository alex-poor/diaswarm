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
}
