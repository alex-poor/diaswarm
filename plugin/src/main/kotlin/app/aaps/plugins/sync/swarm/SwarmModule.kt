package app.aaps.plugins.sync.swarm

import dagger.Module
import dagger.Provides
import javax.inject.Singleton

/**
 * Dagger wiring for the swarm add-on.
 *
 * Deliberately tiny, and deliberately not binding `DataSyncSelectorSwarmImpl`
 * to any interface AAPS already knows about. The selector is reached through
 * this plugin alone, so nothing in the app can acquire it by asking for a
 * general `DataSyncSelector` and get a surprise.
 */
@Module
class SwarmModule {

    @Provides
    @Singleton
    fun providesSwarmSelector(impl: DataSyncSelectorSwarmImpl): DataSyncSelectorSwarmImpl = impl
}
