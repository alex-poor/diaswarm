package app.aaps.plugins.sync.swarm

import android.content.Context
import androidx.work.ExistingPeriodicWorkPolicy
import androidx.work.ExistingWorkPolicy
import androidx.work.OneTimeWorkRequest
import androidx.work.PeriodicWorkRequest
import androidx.work.WorkManager
import java.util.concurrent.TimeUnit
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
import app.aaps.core.interfaces.plugin.PluginBaseWithPreferences
import app.aaps.core.interfaces.plugin.PluginDescription
import app.aaps.core.data.plugin.PluginType
import app.aaps.core.interfaces.resources.ResourceHelper
import app.aaps.core.keys.interfaces.Preferences
import app.aaps.core.validators.preferences.AdaptiveClickPreference
import app.aaps.core.validators.preferences.AdaptiveStringPreference
import app.aaps.plugins.sync.swarm.keys.SwarmLongKey
import app.aaps.plugins.sync.swarm.keys.SwarmStringKey
import androidx.preference.PreferenceCategory
import androidx.preference.PreferenceManager
import androidx.preference.PreferenceScreen
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
    preferences: Preferences,
) : PluginBaseWithPreferences(
    PluginDescription()
        .mainType(PluginType.SYNC)
        .pluginName(R.string.swarm)
        .shortName(R.string.swarm_shortname)
        .description(R.string.description_swarm)
        // Off. Deliberately. See the class comment.
        .enableByDefault(false)
        .visibleByDefault(false)
        // The screen is built in [addPreferenceScreen], not in XML — the XML
        // path in this tree is dead code. Setting this is also what makes the
        // gear icon appear next to the plugin in the Config Builder.
        .preferencesId(PluginDescription.PREFERENCE_SCREEN),
    // REGISTERING THE KEYS IS NOT BOOKKEEPING. Unregistered keys are invisible
    // to `Preferences.get(String)`, are silently dropped from settings export,
    // and in this fork — where the preference list is rendered by Compose off
    // the typed key — render as a dead row. The high-water marks are here too
    // so that a settings export actually carries them.
    ownPreferences = listOf(SwarmLongKey::class.java, SwarmStringKey::class.java),
    aapsLogger, rh, preferences
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
        if (isEnabled()) {
            repairWraps()
            startServing()
            schedulePeriodicSync()
            // SYNC ONCE, NOW. The two-minute poll re-arms itself at the end of
            // a sync pass, so until one has run there is nothing to re-arm and
            // the only thing that will start the chain is the fifteen-minute
            // periodic job. Watched on a follower after an update: serving,
            // enabled, following somebody, and silent for a quarter of an hour
            // — which looks exactly like the person it follows having no data.
            enqueue()
        }
        disposable += rxBus.toObservable(EventNewBG::class.java)
            .observeOn(aapsSchedulers.io)
            .subscribe({ if (isEnabled()) enqueue() }, fabricPrivacy::logException)
        disposable += rxBus.toObservable(EventNewHistoryData::class.java)
            .observeOn(aapsSchedulers.io)
            .subscribe({ if (isEnabled()) enqueue() }, fabricPrivacy::logException)
    }

    /**
     * The settings screen: share, withdraw, and see who can read.
     *
     * WHY THIS EXISTS AT ALL. Granting used to mean force-stopping a running
     * insulin loop, editing an XML file over adb as root, and restarting it.
     * That is not a thing to ask of anyone, including the person who wrote it,
     * and it meant the whole design could only ever be used by one person.
     *
     * Granting still goes through the same preference the sync pass acts on,
     * rather than calling the vault from here. One code path changes access,
     * it runs on the worker thread, and this screen only writes a string to it
     * — a settings screen that reached into the vault directly would be a
     * second way to grant, on the UI thread, that the sync pass knew nothing
     * about.
     */
    override fun addPreferenceScreen(
        preferenceManager: PreferenceManager,
        parent: PreferenceScreen,
        context: Context,
        requiredKey: String?
    ) {
        // Only when the whole screen is being built, not while drilling into
        // some other plugin's sub-screen.
        if (requiredKey != null) return

        val category = PreferenceCategory(context)
        parent.addPreference(category)
        category.apply {
            key = "swarm_settings"
            title = rh.gs(R.string.swarm)
            initialExpandedChildrenCount = 0

            addPreference(
                AdaptiveClickPreference(
                    ctx = context,
                    stringKey = SwarmStringKey.ShowInvite,
                    title = R.string.swarm_show_invite,
                    summary = R.string.swarm_show_invite_summary,
                    onPreferenceClickListener = {
                        SwarmSharing.showInvite(context, currentInvite(), inviteBlockedBecause())
                        true
                    }
                )
            )
            addPreference(
                AdaptiveStringPreference(
                    ctx = context,
                    stringKey = SwarmStringKey.GrantReader,
                    title = R.string.swarm_grant,
                    summary = R.string.swarm_grant_summary
                )
            )
            addPreference(
                AdaptiveStringPreference(
                    ctx = context,
                    stringKey = SwarmStringKey.RevokeReader,
                    title = R.string.swarm_revoke,
                    summary = R.string.swarm_revoke_summary
                )
            )
            addPreference(
                AdaptiveClickPreference(
                    ctx = context,
                    stringKey = SwarmStringKey.ShowReaders,
                    title = R.string.swarm_readers,
                    summary = R.string.swarm_readers_summary,
                    onPreferenceClickListener = {
                        SwarmSharing.showReaders(context, grantedReaders())
                        true
                    }
                )
            )

            // --- the other direction: following someone else ---------------
            addPreference(
                AdaptiveClickPreference(
                    ctx = context,
                    stringKey = SwarmStringKey.ScanCode,
                    title = R.string.swarm_scan,
                    summary = R.string.swarm_scan_summary,
                    onPreferenceClickListener = {
                        context.startActivity(
                            android.content.Intent(context, SwarmScanActivity::class.java)
                        )
                        true
                    }
                )
            )
            addPreference(
                AdaptiveClickPreference(
                    ctx = context,
                    stringKey = SwarmStringKey.ShowFollowing,
                    title = R.string.swarm_following,
                    summary = R.string.swarm_following_summary,
                    onPreferenceClickListener = {
                        // Kick a refresh as well as showing what is held. Opening
                        // this is the one moment someone is definitely waiting for
                        // an answer, and the periodic job could be 15 minutes away.
                        // The dialog still shows what is on disk NOW, with its age,
                        // rather than pretending to have waited for the fetch.
                        enqueue()
                        SwarmSharing.showFollowing(context, following())
                        true
                    }
                )
            )
        }
    }

    /**
     * The invite, or empty if one cannot be made yet.
     *
     * Needs both halves: the subject key, which exists once anything has been
     * sealed, and an endpoint id, which exists only while this node is serving.
     */
    private fun currentInvite(): String {
        if (serving == 0L) return ""
        // Deliberately does NOT require this phone to have sealed anything.
        // A follower's invite is how it hands over the key you must grant.
        SwarmNative.check()
        val subject = SwarmNative.vaultSubject(SwarmPaths.identity(context).absolutePath)
        val endpoint = SwarmNative.swarmNodeId(serving)
        if (subject.isEmpty() || endpoint.isEmpty()) return ""
        return SwarmNative.inviteFor(subject, endpoint, DataSyncSelectorSwarmImpl.PURPOSE)
    }

    /** Why there is no invite, in words a person can act on. */
    private fun inviteBlockedBecause(): String? =
        if (serving == 0L) rh.gs(R.string.swarm_invite_not_ready) else null

    /**
     * What this phone follows, each with the freshest thing it can open.
     *
     * Rows of `subject<TAB>purpose<TAB>reached<TAB>mgdl<TAB>millis`, with the
     * reading fields empty when nothing opens. Assembled here because it needs
     * both the store and this phone's identity, and the dialog should be handed
     * something it can render rather than the means to go looking.
     */
    private fun following(): String {
        SwarmNative.check()
        val store = SwarmPaths.store(context).absolutePath
        val identity = SwarmPaths.identity(context).absolutePath
        return SwarmNative.netFollowing(store).lines().filter { it.isNotBlank() }.joinToString("\n") { row ->
            val f = row.split('\t')
            val subject = f.getOrElse(0) { "" }
            val purpose = f.getOrElse(1) { "" }
            val latest = if (f.getOrElse(2) { "0" } == "1") {
                SwarmNative.netLatest(store, subject, identity, purpose)
            } else ""
            "$row\t$latest"
        }
    }

    private fun grantedReaders(): String {
        SwarmNative.check()
        val vault = SwarmPaths.vault(context, this::class.java)
        if (!java.io.File(vault, "meta.json").exists()) return ""
        return SwarmNative.vaultReaders(vault.absolutePath)
    }

    override fun onStop() {
        disposable.clear()
        WorkManager.getInstance(context).cancelUniqueWork(PERIODIC_JOB_NAME)
        stopServing()
        super.onStop()
    }

    /**
     * Catch up any wraps a granted reader is owed but does not have.
     *
     * Sealing keeps these current, so this normally writes nothing. It is here
     * because an earlier build wrapped only at the moment a grant was made, so
     * every segment sealed afterwards — one per day, plus one per rotation —
     * reached the reader as bytes they could not open. Nothing reported it:
     * the follower kept syncing and kept showing the same stale reading.
     *
     * A vault granted by that build has no record of whose key it was (the
     * grant log names nobody, by design), so this cannot repair those on its
     * own; re-applying the grant with the reader's key does, and no longer
     * appends to the log when the grant already stands.
     */
    private fun repairWraps() {
        SwarmNative.check()
        val vault = SwarmPaths.vault(context, this::class.java)
        if (!java.io.File(vault, "meta.json").exists()) return
        when (val n = SwarmNative.vaultRewrap(vault.absolutePath)) {
            0L -> Unit
            in 1..Long.MAX_VALUE -> aapsLogger.info(LTag.CORE, "swarm: wrapped $n segments readers were owed")
            else -> aapsLogger.warn(LTag.CORE, "swarm: could not bring wraps up to date ($n)")
        }
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

        // SERVE EVEN WITH NOTHING OF OUR OWN TO SERVE.
        //
        // This used to return early unless this phone had sealed something,
        // which quietly made a follower impossible. A second phone that only
        // follows never seals, so it never served, so it had no endpoint id,
        // so it could not produce an invite — and an invite is how it hands
        // over the key you have to grant. The person who most needs to be
        // reachable was the one guaranteed not to be.
        //
        // It is also wrong on its own terms: the STORE is what gets served,
        // not this phone's vault, and a store can be full of other people's
        // history while this phone has published nothing. Serving an empty one
        // costs an idle endpoint.
        // JOIN THE POOL, which also serves what we hold.
        //
        // This replaced starting a bare endpoint of our own. The difference is
        // that a pool member finds other peers instead of waiting to be handed
        // an address, works out which slice of the subject space is its share,
        // and holds what falls there — for people it has never met and cannot
        // read. Serving is the same protocol it always was, registered on
        // p2panda's endpoint.
        serving = SwarmNative.swarmJoin(
            SwarmPaths.store(context).absolutePath,
            SwarmPaths.nodeKey(context).absolutePath
        )
        if (serving == 0L) {
            aapsLogger.error(LTag.CORE, "swarm: could not start serving")
            return
        }
        SwarmEndpoint.handle = serving
        aapsLogger.info(LTag.CORE, "swarm: in the pool as ${SwarmNative.swarmNodeId(serving)}")
        aapsLogger.info(
            LTag.CORE,
            "swarm: subject ${SwarmNative.vaultSubject(SwarmPaths.identity(context).absolutePath)}"
        )
        val vault = SwarmPaths.vault(context, this::class.java)
        if (!java.io.File(vault, "meta.json").exists()) {
            aapsLogger.info(LTag.CORE, "swarm: nothing sealed here yet — following only")
        }
    }

    private fun stopServing() {
        SwarmEndpoint.handle = 0L
        if (serving == 0L) return
        SwarmNative.swarmLeave(serving)
        serving = 0L
        aapsLogger.info(LTag.CORE, "swarm: left the pool")
    }

    /**
     * A heartbeat, because a follower has nothing of its own to react to.
     *
     * THE EVENT TRIGGERS ONLY FIRE ON A PHONE THAT IS LOOPING. `EventNewBG` and
     * `EventNewHistoryData` come from a CGM and a pump, so a device that only
     * follows someone else — no sensor, no pump, nothing writing to its
     * database — never enqueued a sync at all. It would accept an invite, sit
     * there, and fetch nothing, for ever. That is the exact shape of failure
     * §12.3 is about: it looks like the other person has no data.
     *
     * Fifteen minutes is WorkManager's floor for periodic work, not a chosen
     * number. The event triggers stay, because a looping phone should publish
     * promptly rather than up to a quarter of an hour late.
     */
    private fun schedulePeriodicSync() {
        WorkManager.getInstance(context).enqueueUniquePeriodicWork(
            PERIODIC_JOB_NAME,
            ExistingPeriodicWorkPolicy.UPDATE,
            PeriodicWorkRequest.Builder(SwarmDataSyncWorker::class.java, 15, TimeUnit.MINUTES).build()
        )
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
        const val PERIODIC_JOB_NAME = "SwarmDataSyncPeriodic"

        /**
         * The fast poll a following phone re-arms after every pass. Separate
         * from [JOB_NAME] so that a publish triggered by new CGM data and a
         * follower's next poll cannot cancel one another.
         */
        const val FOLLOW_JOB_NAME = "SwarmFollowPoll"
    }
}
