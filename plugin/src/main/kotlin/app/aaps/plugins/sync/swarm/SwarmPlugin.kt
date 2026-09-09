package app.aaps.plugins.sync.swarm

import android.content.Context
import androidx.work.ExistingWorkPolicy
import androidx.work.OneTimeWorkRequest
import androidx.work.WorkManager
import app.aaps.core.interfaces.logging.AAPSLogger
import app.aaps.core.interfaces.logging.LTag
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
    /** The running endpoint, while enabled. 0 when not serving. */
    private var serving: Long = 0L

    override fun onStart() {
        super.onStart()
        if (isEnabled()) startServing()
        disposable += rxBus.toObservable(EventNewBG::class.java)
            .observeOn(aapsSchedulers.io)
            .subscribe({ if (isEnabled()) enqueue() }, fabricPrivacy::logException)
        disposable += rxBus.toObservable(EventNewHistoryData::class.java)
            .observeOn(aapsSchedulers.io)
            .subscribe({ if (isEnabled()) enqueue() }, fabricPrivacy::logException)
    }

    override fun onStop() {
        disposable.clear()
        stopServing()
        super.onStop()
    }

    /**
     * Serve the vault to peers.
     *
     * Only while enabled, and stopped on the way out. A disabled plugin that
     * left a network endpoint listening would be exactly the surprise
     * `enableByDefault(false)` exists to prevent.
     *
     * The endpoint id and the subject key are logged because there is nowhere
     * else yet to see them, and without both a reader cannot fetch anything:
     * one says where, the other is what a grant is made against.
     */
    private fun startServing() {
        if (serving != 0L) return
        SwarmNative.check()
        val vault = SwarmPaths.vault(context, this::class.java)
        if (!java.io.File(vault, "meta.json").exists()) {
            aapsLogger.info(LTag.CORE, "swarm: nothing sealed yet, not serving")
            return
        }
        // The STORE is served, not this vault: whatever this node has
        // replicated from other subjects is served onward too, which is what
        // makes it a peer rather than a personal server.
        serving = SwarmNative.netStart(
            SwarmPaths.store(context).absolutePath,
            SwarmPaths.nodeKey(context).absolutePath
        )
        if (serving == 0L) {
            aapsLogger.error(LTag.CORE, "swarm: could not start serving")
            return
        }
        aapsLogger.info(LTag.CORE, "swarm: serving as ${SwarmNative.netEndpointId(serving)}")
        aapsLogger.info(
            LTag.CORE,
            "swarm: subject ${SwarmNative.vaultSubject(SwarmPaths.identity(context).absolutePath)}"
        )
    }

    private fun stopServing() {
        if (serving == 0L) return
        SwarmNative.netStop(serving)
        serving = 0L
        aapsLogger.info(LTag.CORE, "swarm: stopped serving")
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
