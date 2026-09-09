package app.aaps.plugins.sync.swarm.workers

import android.content.Context
import androidx.work.WorkerParameters
import app.aaps.core.objects.workflow.LoggingWorker
import app.aaps.plugins.sync.swarm.DataSyncSelectorSwarmImpl
import kotlinx.coroutines.Dispatchers
import javax.inject.Inject

/**
 * Runs the drain.
 *
 * WITHOUT THIS THE PLUGIN DOES NOTHING, and that is not obvious from reading it:
 * a `DataSyncSelector` is an interface AAPS calls, not a thing that runs itself.
 * Enabling a plugin that implements one and never enqueues a worker gives you a
 * checkbox that ticks and no other effect — which is exactly what happened here
 * before this file existed.
 *
 * Off the main thread, on IO: sealing does public-key work and writes files, and
 * neither belongs anywhere near the loop's own thread.
 */
class SwarmDataSyncWorker(
    context: Context,
    params: WorkerParameters
) : LoggingWorker(context, params, Dispatchers.IO) {

    @Inject lateinit var dataSyncSelector: DataSyncSelectorSwarmImpl

    override suspend fun doWorkAndLog(): Result {
        dataSyncSelector.doUpload()
        return Result.success()
    }
}
