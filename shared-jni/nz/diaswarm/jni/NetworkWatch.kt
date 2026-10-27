// SHARED SOURCE — ONE FILE, TWO BUILDS. This lives in the repo-root `shared-jni/`
// tree and is compiled into BOTH the Ayni follower (`follower/`, its own Gradle
// build) AND the AAPS plugin (`plugin/`, built inside the AndroidAPS tree) via a
// `sourceSets["main"].java.srcDir("../shared-jni")` line in each module's
// build.gradle.kts. Edit it HERE and only here — there is deliberately no second
// copy. It once existed twice and the plugin copy kept leaking into AndroidAPS
// after the follower copy was fixed (diaswarm e982c7e fixed one, 70567be the
// other). Keep it dependency-free beyond the Android SDK so both builds can take
// it as-is. SwarmNative.kt is intentionally NOT shared — the plugin exposes
// publisher-only JNI the follower does not.
package nz.diaswarm.jni

import android.content.Context
import android.net.ConnectivityManager
import android.net.LinkProperties
import android.net.Network
import android.net.NetworkCapabilities
import java.util.concurrent.atomic.AtomicBoolean

/**
 * Tells the swarm when Android changes the network under it.
 *
 * **THIS CLASS IS A 71-MINUTE OUTAGE WRITTEN DOWN.** On 2026-09-15 the loop
 * phone left the house at 15:05. Wifi became mobile data, its relay connection
 * went, and it did not come back until 16:16 when wifi returned — seventy-one
 * minutes during which the phone was unreachable from every other network while
 * looking perfectly healthy from its own side. AAPS never faltered: it sealed
 * eighty-four epochs into that hole, `holds` climbed 1470 -> 1560, with
 * `missing 0, lost 0, failures 0`. The data was made and it was kept. It simply
 * had nowhere to go.
 *
 * **IT WAS NOT DOZE, NOT THE CARRIER, AND NOT THE FOREGROUND SERVICE** — three
 * things previously blamed for overnight failures, and none of them this. The
 * cause is that iroh cannot see an Android network change. `netwatch` ships a
 * deliberately empty route monitor for this platform, whose entire body is a
 * comment reading *"Very sad monitor. Android doesn't allow us to do this"*, and
 * its wall-time poll is stretched to an hour on mobile to save battery — and
 * fires on a *clock* jump, never a network one. So sockets stay bound to the
 * interface the phone just walked away from, and nothing ever says otherwise.
 *
 * iroh's own documentation names the remedy and names this platform:
 *
 * > some systems like android do not expose this functionality to native code.
 * > Android does however provide this functionality to Java code.
 *
 * Java is here. This is the part we had never written.
 *
 * **IT NOTIFIES ONLY WHEN THE NETWORK IDENTITY ACTUALLY CHANGES.** Upstream says
 * "even when the network did not change [...] there is no harm in calling this
 * function" — and on this follower there is: each call rebinds the iroh endpoint
 * and leaks ~1 MB of ring / connection state. Android delivers capability and
 * link-property callbacks every few seconds (signal strength, validation,
 * metering) on a network that has not actually moved, so notifying on every one
 * leaked ~20 MB/min and drove the follower past a 1 GB heap in half an hour
 * (observed live 2026-10-27).
 *
 * So what the sockets actually depend on is fingerprinted — the default
 * network's id, its transports, its interface and its link addresses — and a
 * notification fires only when that fingerprint changes. A real handover
 * (wifi<->cellular, a new DHCP lease, a network lost or gained) still changes it
 * and still notifies, so the 71-minute outage this class exists to prevent stays
 * prevented; a signal-strength flicker on the same network no longer does.
 * Bursts are still coalesced to one in-flight notification.
 */
object NetworkWatch {

    private var registered: ConnectivityManager.NetworkCallback? = null

    /**
     * Set once [stop] has run, and checked inside the lock before the handle is
     * used.
     *
     * **BECAUSE THE HANDLE IS A RAW POINTER AND LEAVING FREES IT.**
     * `swarmLeave` does `Box::from_raw(..)` and drops. Both apps zero their
     * Kotlin handle before calling it, but a notification that had already read
     * a live handle would then dereference freed memory — and unlike every other
     * caller, this one is driven by an OS callback that can fire at any instant,
     * including during teardown. On the phone that drives an insulin pump that
     * is not a race worth leaving narrow.
     */
    @Volatile
    private var stopped = false

    /** How long a burst of callbacks is allowed to settle before notifying. */
    private const val COALESCE_MS = 1_500L

    /** How often a notification is written to the log, however many arrive. */
    private const val LOG_EVERY_MS = 60_000L
    private var lastLoggedAt = 0L
    private var countSinceLogged = 0

    /**
     * The last network the sockets were told about, as a fingerprint of the
     * things they depend on. A callback whose fingerprint equals this one is a
     * flicker on the same network — not a move — and must not rebind the
     * endpoint, because rebinding leaks (see the class comment). Guarded by
     * [stateLock], a plain lock rather than the [NetworkWatch] monitor, so a
     * callback on a binder thread never blocks behind a running notification's
     * JNI call.
     */
    private val stateLock = Any()
    private var curNetwork: Network? = null
    private var curCaps: NetworkCapabilities? = null
    private var curProps: LinkProperties? = null
    private var lastKey: String? = null

    /** What the sockets depend on: which network, its transports, interface and addresses — NOT signal strength or validation, which churn without moving anything. */
    private fun fingerprint(net: Network?, caps: NetworkCapabilities?, props: LinkProperties?): String {
        if (net == null) return "none"
        val transports = (0..7).filter { caps?.hasTransport(it) == true }.joinToString(",")
        val iface = props?.interfaceName ?: "?"
        val addrs = props?.linkAddresses?.map { it.toString() }?.sorted()?.joinToString(",") ?: "?"
        return "$net|$transports|$iface|$addrs"
    }

    /**
     * Start watching, notifying the swarm [handle] gives back on every change.
     *
     * A supplier rather than a value because the two apps each hold their handle
     * in their own singleton, and because a handle read at registration time
     * would be the one from before the last restart. Safe to call repeatedly;
     * the second call is a no-op.
     *
     * **[log] IS INJECTED SO THIS LANDS SOMEWHERE THAT SURVIVES.** The outage
     * above was only diagnosable because AAPS keeps a durable log on disk;
     * logcat's ring buffer had long since rolled past it. Native `Log` would put
     * these lines only in the buffer that loses them, so each app passes the
     * writer that keeps them.
     */
    @Synchronized
    fun start(context: Context, log: (String) -> Unit, handle: () -> Long) {
        if (registered != null) return
        stopped = false
        // Zero so the first change after a (re)start is always logged.
        lastLoggedAt = 0L
        countSinceLogged = 0
        // Forget the last network, so the first callback after a (re)start always
        // counts as a change and tells the swarm once.
        synchronized(stateLock) { curNetwork = null; curCaps = null; curProps = null; lastKey = null }
        val manager = context.getSystemService(ConnectivityManager::class.java) ?: run {
            log("network watch: no ConnectivityManager — changes will go unnoticed")
            return
        }
        // **NOT ON THE CALLBACK THREAD.** `swarmNetworkChanged` blocks on the
        // runtime, and this callback arrives on a binder thread that Android
        // expects back promptly.
        val busy = AtomicBoolean(false)
        val notify = {
            if (busy.compareAndSet(false, true)) {
                Thread({
                    try {
                        // **TRAILING EDGE, NOT LEADING.** Android delivers these
                        // in bursts — measured on the loop phone, one every ~3
                        // seconds — because capability and link-property changes
                        // count, not just handovers. Notifying on each would
                        // spawn a thread and cross JNI every few seconds all day,
                        // and battery cost is a thing users get to refuse.
                        //
                        // Sleeping FIRST is what makes coalescing safe: every
                        // callback arriving during the wait is dropped, but the
                        // one notification lands after them and therefore sees
                        // the settled network. Dropping the *last* callback of a
                        // burst is the one mistake that would reintroduce the
                        // bug, and this cannot make it.
                        Thread.sleep(COALESCE_MS)
                        // READ AND USE THE HANDLE UNDER THE SAME LOCK [stop]
                        // takes, so teardown cannot free it in between.
                        synchronized(NetworkWatch) {
                            val h = if (stopped) 0L else handle()
                            if (h != 0L) {
                                SwarmNative.swarmNetworkChanged(h)
                                countSinceLogged++
                                // **NOTIFY EVERY TIME, LOG ONCE A MINUTE.**
                                // Android delivers these in a steady trickle —
                                // capability and link-property changes, not just
                                // real handovers — and measured on the loop
                                // phone that is a line every twenty seconds.
                                // Left unchecked it is ~3,000 lines a day
                                // pushing history out of AndroidAPS.log, which
                                // is the log the outage behind this class was
                                // diagnosed from. The count is kept so the
                                // suppressed ones are still visible.
                                val now = System.currentTimeMillis()
                                if (now - lastLoggedAt >= LOG_EVERY_MS) {
                                    val n = countSinceLogged
                                    lastLoggedAt = now
                                    countSinceLogged = 0
                                    log(
                                        if (n == 1) "network watch: told the swarm the network changed"
                                        else "network watch: told the swarm the network changed ($n times)"
                                    )
                                }
                            }
                        }
                    } catch (e: Throwable) {
                        log("network watch: could not notify: $e")
                    } finally {
                        busy.set(false)
                    }
                }, "diaswarm-netwatch").start()
            }
        }
        // **NOTIFY ONLY ON A REAL CHANGE.** Each callback updates the cached
        // picture of the default network and recomputes the fingerprint; `notify`
        // fires only when it differs from the last network the sockets were told
        // about. A signal-strength or validation callback on the same network
        // leaves the fingerprint unchanged and costs nothing — which is what
        // stops the rebind-per-flicker leak.
        fun onChange(mutate: () -> Unit) {
            val changed = synchronized(stateLock) {
                mutate()
                val key = fingerprint(curNetwork, curCaps, curProps)
                (key != lastKey).also { if (it) lastKey = key }
            }
            if (changed) notify()
        }
        val callback = object : ConnectivityManager.NetworkCallback() {
            override fun onAvailable(network: Network) = onChange { curNetwork = network }
            override fun onLost(network: Network) = onChange {
                if (curNetwork == network) { curNetwork = null; curCaps = null; curProps = null }
            }
            override fun onCapabilitiesChanged(network: Network, caps: NetworkCapabilities) =
                onChange { curNetwork = network; curCaps = caps }
            override fun onLinkPropertiesChanged(network: Network, props: LinkProperties) =
                onChange { curNetwork = network; curProps = props }
        }
        try {
            // THE DEFAULT NETWORK, because that is the one the sockets use. A
            // request-based callback would also fire for networks nothing is
            // routed over.
            manager.registerDefaultNetworkCallback(callback)
            registered = callback
            log("network watch: registered")
        } catch (e: Throwable) {
            log("network watch: could not register: $e")
        }
    }

    /**
     * Stop watching, and make sure nothing is still inside a notification.
     *
     * **CALL THIS BEFORE `swarmLeave`.** It is `@Synchronized`, so it cannot
     * return while a notification holds the lock, and it sets [stopped] so a
     * callback already queued finds nothing to do. After it returns the handle
     * is safe to free.
     */
    @Synchronized
    fun stop(context: Context, log: (String) -> Unit) {
        stopped = true
        val callback = registered ?: return
        registered = null
        try {
            context.getSystemService(ConnectivityManager::class.java)?.unregisterNetworkCallback(callback)
        } catch (e: Throwable) {
            log("network watch: could not unregister: $e")
        }
    }
}
