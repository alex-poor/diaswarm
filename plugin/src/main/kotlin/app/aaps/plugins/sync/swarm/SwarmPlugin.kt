package app.aaps.plugins.sync.swarm

import android.content.Context
import androidx.work.ExistingWorkPolicy
import androidx.work.OneTimeWorkRequest
import androidx.work.WorkManager
import app.aaps.core.interfaces.logging.AAPSLogger
import app.aaps.core.interfaces.rx.AapsSchedulers
import app.aaps.core.interfaces.rx.bus.RxBus
import app.aaps.core.interfaces.rx.events.EventNewBG
import app.aaps.core.interfaces.rx.events.EventNewHistoryData
import app.aaps.core.interfaces.utils.fabric.FabricPrivacy
import app.aaps.plugins.sync.swarm.workers.SwarmDataSyncWorker
import io.reactivex.rxjava3.disposables.CompositeDisposable
import io.reactivex.rxjava3.kotlin.plusAssign
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
    private val context: Context,
    private val rxBus: RxBus,
    private val aapsSchedulers: AapsSchedulers,
    private val fabricPrivacy: FabricPrivacy,
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
) {

    private val disposable = CompositeDisposable()

    /**
     * Ask for a drain when new data lands.
     *
     * `beginUniqueWork` with `KEEP`, not `REPLACE`: a drain that is already
     * running is doing the same work this request wants done, and cancelling it
     * mid-seal to start again would rewrite a segment that is being written.
     *
     * Only while enabled. AAPS calls `onStart` on every plugin regardless, so
     * without the guard a disabled plugin would still subscribe and still
     * enqueue — which is the opposite of what shipping disabled is supposed to
     * mean.
     */
    override fun onStart() {
        super.onStart()
        disposable += rxBus.toObservable(EventNewBG::class.java)
            .observeOn(aapsSchedulers.io)
            .subscribe({ if (isEnabled()) enqueue() }, fabricPrivacy::logException)
        disposable += rxBus.toObservable(EventNewHistoryData::class.java)
            .observeOn(aapsSchedulers.io)
            .subscribe({ if (isEnabled()) enqueue() }, fabricPrivacy::logException)
    }

    override fun onStop() {
        disposable.clear()
        super.onStop()
    }

    private fun enqueue() {
        WorkManager.getInstance(context).beginUniqueWork(
            JOB_NAME,
            ExistingWorkPolicy.KEEP,
            OneTimeWorkRequest.Builder(SwarmDataSyncWorker::class.java).build()
        ).enqueue()
    }

    companion object {

        const val JOB_NAME = "SwarmDataSync"
    }
}
