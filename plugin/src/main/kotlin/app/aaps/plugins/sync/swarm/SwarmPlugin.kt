package app.aaps.plugins.sync.swarm

import app.aaps.core.interfaces.logging.AAPSLogger
import app.aaps.core.interfaces.plugin.PluginBase
import app.aaps.core.interfaces.plugin.PluginDescription
import app.aaps.core.data.plugin.PluginType
import app.aaps.core.interfaces.resources.ResourceHelper
import javax.inject.Inject
import javax.inject.Singleton

/**
 * The swarm sync plugin: an add-on, off unless someone turns it on.
 *
 * IT SHIPS DISABLED, AND THAT IS THE WHOLE SAFETY ARGUMENT. This class is
 * compiled into an app that drives an insulin pump. `enableByDefault(false)`
 * means AAPS constructs it and then leaves it alone: the sync framework only
 * calls plugins that are enabled, so with the toggle off nothing here runs,
 * no records are read, and `System.loadLibrary` is never reached — the native
 * library is loaded from [SwarmNative.check], which only `doUpload` calls.
 *
 * WHAT IT CANNOT DO, STRUCTURALLY. It is a `DataSyncSelector`, which drains a
 * queue outward (decision D7). There is no path from this class into dosing, it
 * holds no pump reference, it implements no constraint, and it must never
 * acquire any of those. Anything that can influence dosing is inside the medical
 * device's blast radius and inherits its entire risk posture.
 *
 * THE RESIDUAL RISK IS APP START, NOT DOSING. A plugin that fails to construct
 * takes the Dagger graph with it, and an app that will not start is a loop that
 * has stopped. That is why the constructor takes nothing but a logger and a
 * resource helper, does no work, touches no file, and loads no library.
 */
@Singleton
class SwarmPlugin @Inject constructor(
    aapsLogger: AAPSLogger,
    rh: ResourceHelper,
) : PluginBase(
    PluginDescription()
        .mainType(PluginType.SYNC)
        .pluginName(R.string.swarm)
        .shortName(R.string.swarm_shortname)
        .description(R.string.description_swarm)
        // Off. Deliberately. See the class comment.
        .enableByDefault(false)
        .visibleByDefault(false),
    aapsLogger, rh
)
