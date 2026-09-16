package nz.diaswarm.follower

import android.content.Context
import android.util.Log
import androidx.work.Worker
import androidx.work.WorkerParameters
import nz.diaswarm.jni.SwarmNative

/**
 * Ask the people this phone follows whether anything has changed.
 *
 * **THE CADENCE IS THE DATA'S, NOT A PREFERENCE.** A CGM produces a reading
 * every one to five minutes, so polling faster mostly discovers that nothing has
 * changed — which still costs a connection. Two minutes sits under the data's
 * own rhythm without polling into the gaps.
 *
 * ANDROID DECIDES THE REST. Doze batches background work, so "two minutes" means
 * two minutes while the phone is awake and something longer in a pocket. That is
 * the platform, not the design, and it is exactly why every reading on screen
 * carries its age rather than appearing as a bare number that implies it is
 * current.
 */
class SyncWorker(context: Context, params: WorkerParameters) : Worker(context, params) {

    override fun doWork(): Result {
        return try {
            SwarmNative.check()
            val store = SwarmPaths.store(applicationContext).absolutePath

            // Act on anything the user asked for before fetching, so a freshly
            // pasted invite is followed in this pass rather than the next one.
            Prefs.let { }
            val pending = Invites.takePending(applicationContext)
            if (pending.isNotEmpty()) {
                val n = SwarmNative.netFollow(store, pending)
                Log.i(TAG, "followed an invite: $n")
                // **LEARN THE NAME HERE, BECAUSE THIS IS THE ONLY TIME IT IS
                // OFFERED.** The handle travels in the invite and nowhere else
                // — it is never published — so an invite acted on and discarded
                // without reading it leaves this device with sixteen hex
                // characters forever. Stored against the subject key, which is
                // the identity; the name is only the label. Empty for every
                // invite issued before v4, and for anyone who has not named
                // themselves.
                val fields = SwarmNative.inviteParse(pending).split('\t')
                val subject = fields.getOrElse(0) { "" }
                val handle = fields.getOrElse(5) { "" }
                if (subject.isNotEmpty() && handle.isNotEmpty() &&
                    Prefs.handleFor(applicationContext, subject).isEmpty()
                ) {
                    // Only if this device has not already been told otherwise:
                    // a name the person edited is theirs, not the invite's.
                    Prefs.setHandleFor(applicationContext, subject, handle)
                    Log.i(TAG, "learned a name for ${subject.take(8)}")
                }
            }

            if (Follower.following(applicationContext).isEmpty()) {
                Log.i(TAG, "following nobody — nothing to fetch")
                return Result.success()
            }
            // **BEFORE ANYTHING DIALS, TAKE A TURN IN THE POOL.**
            //
            // This app joined the pool and then never participated: `swarmTick`
            // was declared in `SwarmNative` and called from nowhere. Joining
            // gets you a node id; the tick is what announces what you hold,
            // hears who holds what, and — the part that matters here — keeps a
            // live view of where the people you follow actually are.
            //
            // Without it a follower can only reach a subject while the address
            // it learned when the invite was scanned still works. It does, for
            // as long as nothing moves. Found after one night: the subject's
            // phone slept and reconnected, and this app went blind and STAYED
            // blind — a fresh process did not fix it, because a fresh process
            // joined the pool and did not tick either. Both vaults went empty
            // together, which is what said it was not a vault problem.
            //
            // The AAPS plugin has always ticked once per pass, which is why the
            // same phone could reach the same subject from the other app at the
            // same moment.
            //
            // ADOPT, LIKE EVERY OTHER PEER. This passed `0` for a day, from a
            // session that was fixing discovery and did not want to decide
            // this in passing — "whether a follower's phone should start
            // holding strangers' ciphertext is a separate decision".
            //
            // **THE DECISION WAS ALREADY MADE, AND IT IS THE APP'S NAME.**
            // `strings.xml`: *Ayni* is Quechua for reciprocity, "your phone
            // carries other people's sealed records so that yours are carried
            // when your phone is off, and neither side can read what it
            // holds." A follower that adopts nothing takes that bargain and
            // keeps only the half that benefits it — and, worse, it is the
            // half that does not work: a subject is highly available because
            // other phones hold it, and if every follower holds nothing then
            // the only peers carrying anything are the subjects themselves.
            // That is not a swarm, it is a set of servers with extra steps.
            //
            // Same budget as the plugin. Bounded per pass, so a phone joining
            // a large pool catches up over passes rather than pulling its
            // whole share at once.
            if (Endpoint.handle != 0L) {
                val pool = try {
                    SwarmNative.swarmTick(Endpoint.handle, 2)
                } catch (e: Throwable) {
                    Log.w(TAG, "pool pass threw: $e")
                    ""
                }
                Log.i(TAG, if (pool.isEmpty()) "pool pass failed" else "pool $pool")
            }

            val reached = SwarmNative.netRefresh(Endpoint.handle, store)
            Log.i(TAG, "refreshed $reached subject(s)")

            // **CARRY THE KEYS LOGS TOO, AND SAY HOW MANY.** Without this the
            // replicator subscribes to nothing and every keys read finds an
            // empty store, which looks exactly like "not granted yet". Logged
            // either way: a pass that carried nothing and a pass that carried
            // everything must not read the same.
            if (Prefs.keysVault(applicationContext) && Endpoint.handle != 0L) {
                val carried = SwarmNative.keysCarryAll(
                    Endpoint.handle,
                    store,
                    SwarmPaths.identity(applicationContext).absolutePath,
                    // The same budget this pass gives `swarmTick`, and for the
                    // reason recorded there: carrying other people's sealed
                    // records is what this app is named after.
                    2
                )
                Log.i(TAG, if (carried < 0) "keys carry unavailable ($carried)" else "keys carrying $carried log(s)")
                // ON SCREEN, NOT ONLY IN THE LOG. See [Prefs.carrying].
                if (carried >= 0) Prefs.setCarrying(applicationContext, carried.toInt())
                verifyChains(applicationContext, keysToo = carried > 0)
                if (carried > 0) watchForAStall(applicationContext)

                // **OFFER OUR KEYS IDENTITY TO PEOPLE WHO ALREADY GRANTED US.**
                // They granted this phone on the old vault, possibly months
                // ago; the new one needs an identity they have never seen, and
                // only we can supply it with a proof only the two of us can
                // make. Nobody scans anything. A subject that has not moved to
                // the keys vault, or is too old to understand the request,
                // answers 0 — and that is the status quo, not a failure.
                val identity = SwarmKeys.identity(applicationContext)
                if (identity.isNotEmpty()) {
                    for (subject in Follower.following(applicationContext)) {
                        // Only where we have something to hand over to: a
                        // subject with no keys identity of their own has no
                        // keys vault to be granted on.
                        if (subject.keys.isEmpty()) continue
                        // AND NOT EVERY PASS. See [Prefs.handedOverAt]: this
                        // was asking once a minute, for ever, and the subject
                        // was granting once a minute, for ever.
                        val since = System.currentTimeMillis() -
                            Prefs.handedOverAt(applicationContext, subject.key)
                        if (since < HANDOVER_RETRY_MS) continue
                        val took = SwarmNative.netHandOver(
                            Endpoint.handle,
                            store,
                            SwarmPaths.identity(applicationContext).absolutePath,
                            subject.key,
                            identity
                        )
                        if (took == 1L) {
                            Prefs.setHandedOverAt(applicationContext, subject.key, System.currentTimeMillis())
                            Log.i(TAG, "handed over to ${subject.short}")
                        }
                    }
                }
            }
            Result.success()
        } catch (e: Throwable) {
            // An unreachable peer is the ordinary condition of a swarm, not an
            // error. The age on screen is what tells somebody it has gone on
            // too long.
            Log.w(TAG, "sync pass failed", e)
            Result.success()
        }
    }

    /**
     * Check each followed subject's control log still holds together.
     *
     * **THE POINT OF A TAMPER-EVIDENT LOG IS SOMEBODY CHECKING IT.** D13's
     * grant log replicates precisely so that truncating or altering it is
     * detectable, and §11 offers that *in place of* a read log — but the code
     * that detects it had never run outside a test on either phone. A property
     * nobody evaluates is a claim, not a property.
     *
     * Here rather than on the subject's phone because the subject cannot catch
     * themselves: this runs on the copy this follower was actually served.
     *
     * Once per subject per pass, on a log that holds one entry per grant and
     * revocation — a handful of hashes, not a thing to be clever about. Logged
     * quietly when intact and loudly when not, and it changes nothing else:
     * a broken chain is something a person has to look at, not something an
     * app should act on by itself.
     */
    /**
     * Notice when the keys vault has stopped receiving, and re-establish.
     *
     * **A SUBSCRIPTION THAT EXISTS IS NOT A SUBSCRIPTION THAT WORKS.** After the
     * subject's phone changed network, log sync wedged and never recovered:
     * eleven minutes with no segment arriving, the row count sliding backwards
     * as the window moved on, and `keys carrying 2 log(s)` printed on every
     * single pass throughout. Force-stopping the app fixed it in one pass. So
     * the remedy is known and cheap — what was missing was anything noticing.
     *
     * `STALE_AFTER` is generous on purpose. A CGM produces a reading a minute,
     * so ten minutes of silence is not a slow network, it is a stall. Being
     * wrong here costs a reconnect; being too eager costs one every pass.
     *
     * `MIN_BETWEEN_RESTARTS` is the guard that matters. A subject whose phone
     * is genuinely off produces exactly the same silence, and restarting the
     * endpoint every two minutes for somebody who is asleep would be this app
     * making its own weather. Once a quarter of an hour heals a stall promptly
     * and costs nothing measurable when the quiet is real.
     */
    private fun watchForAStall(context: Context) {
        val stale = Follower.following(context)
            .filter { it.keys.isNotEmpty() }
            .mapNotNull { s -> Follower.keysNewestAgeMs(context, s)?.let { s to it } }
        if (stale.isEmpty()) return

        for ((subject, age) in stale) {
            Log.i(TAG, "keys newest for ${subject.short}: ${age / 1000}s old")
        }
        // WHAT THE STREAM ACTUALLY DID, not what the age implies it did.
        // The replicator has recorded these since it was written and nothing
        // could read them, so every explanation of a stall so far has been
        // inferred from the outside — and three of them in one hour were wrong.
        val h = SwarmKeys.open(context)
        if (h != 0L) {
            try {
                val events = SwarmNative.keysSyncEvents(Endpoint.handle, 6)
                    .lines().filter { it.isNotBlank() }
                // SAYS SO WHEN THERE ARE NONE. An empty list read as silence is
                // how the first version of this told me nothing for a pass and
                // I nearly concluded the call was broken. Empty is a finding:
                // the replicator records session events and errors but not
                // arrivals, so none at all means no session has happened.
                //
                // **AND THE COMMENT HERE USED TO SAY "everything came by gossip
                // push", WHICH WAS EXACTLY BACKWARDS.** Nothing ever came by
                // push: the publishing side never called `SyncHandle::publish`,
                // so every byte this follower has ever shown arrived in a
                // catch-up sync. The `counts` line now says which it was.
                if (events.isEmpty()) Log.i(TAG, "sync: no session events recorded")
                else events.forEach { Log.i(TAG, "sync: $it") }
                // AND WHETHER ANYBODY ELSE HOLDS THIS SUBJECT. D15's promise is
                // that any holder serves identical bytes, so a subject whose
                // phone is asleep can still be read from a peer — and whether
                // a peer is there has never been visible from a phone. When
                // this follower sat in a pool of one all morning, this is the
                // line that would have said so.
                for ((subject, _) in stale) {
                    val holders = SwarmNative.swarmHoldersHeard(Endpoint.handle, subject.key)
                        .lines().filter { it.isNotBlank() }
                    Log.i(
                        TAG,
                        if (holders.isEmpty()) "holders for ${subject.short}: none — only the subject can serve this"
                        else "holders for ${subject.short}: ${holders.size} (${holders.joinToString { it.take(8) }})"
                    )
                    // AND THE SAME FOR THE VAULT BEING CUT OVER TO, WHICH IS THE
                    // ONE THAT MATTERS NOW. Until the pool carried keys logs
                    // this was "none" by construction: a follower could only
                    // ever reach a subject at the subject's own phone, which is
                    // the opposite of what the pool is for.
                    val keysHolders = SwarmNative.keysHoldersHeard(Endpoint.handle, subject.keys)
                        .lines().filter { it.isNotBlank() }
                    Log.i(
                        TAG,
                        if (keysHolders.isEmpty()) "keys holders for ${subject.short}: none"
                        else "keys holders for ${subject.short}: ${keysHolders.size} (${keysHolders.joinToString { it.take(8) }})"
                    )
                }
            } catch (e: Throwable) {
                Log.w(TAG, "sync events threw: $e")
            } finally {
                SwarmNative.keysClose(h)
            }
        }
        val worst = stale.maxOf { it.second }
        if (worst < STALE_AFTER) return

        val since = System.currentTimeMillis() - Prefs.lastEndpointRestart(context)
        if (since < MIN_BETWEEN_RESTARTS) {
            Log.w(TAG, "keys stalled (${worst / 1000}s) — waiting, last reconnect ${since / 1000}s ago")
            return
        }
        // **RE-SUBSCRIBE FIRST, RESTART ONLY IF THAT FAILS.** The root cause is
        // a one-shot subscription: the stream catches up once and then waits
        // for gossip, and when that link dies nothing re-establishes it.
        // Re-streaming the topic fixes exactly that and costs one reconnection;
        // restarting the endpoint also works and drops every other connection
        // this phone has with it.
        Prefs.setLastEndpointRestart(context, System.currentTimeMillis())
        val handle = SwarmKeys.open(context)
        val restreamed = if (handle == 0L) -1L else try {
            SwarmNative.keysRestream(Endpoint.handle)
        } catch (e: Throwable) {
            Log.w(TAG, "restream threw: $e"); -1L
        } finally {
            SwarmNative.keysClose(handle)
        }
        if (restreamed > 0) {
            Log.e(TAG, "keys stalled for ${worst / 1000}s — re-subscribed $restreamed topic(s)")
        } else {
            Log.e(TAG, "keys stalled for ${worst / 1000}s and re-subscribing gave $restreamed — reconnecting")
            Endpoint.restart(context)
        }
    }

    private fun verifyChains(context: Context, keysToo: Boolean) {
        val handle = if (keysToo) SwarmKeys.open(context) else 0L
        try {
            for (subject in Follower.following(context)) {
                // The core log first, because it is the one that is live today
                // and stays live after a cutover — both vaults are read.
                say(subject.short, "core", runCatching {
                    SwarmNative.vaultVerifyChain(
                        SwarmPaths.store(context).absolutePath,
                        subject.key
                    )
                })
                if (handle != 0L && subject.keys.isNotEmpty()) {
                    say(subject.short, "keys", runCatching {
                        SwarmNative.keysVerifyControl(handle, subject.keys)
                    })
                }
            }
        } finally {
            if (handle != 0L) SwarmNative.keysClose(handle)
        }
    }

    /** One verdict, at the level it deserves. A break is not an info line. */
    // `kotlin.Result`, spelled out: inside a Worker, a bare `Result` is
    // `ListenableWorker.Result` and the file compiles in a different language
    // than the one it looks like.
    private fun say(who: String, which: String, verdict: kotlin.Result<String>) {
        val v = verdict.getOrElse { "error threw $it" }
        when {
            v.startsWith("ok") -> Log.i(TAG, "$which grant log intact for $who: $v")
            v.startsWith("broken") -> Log.e(TAG, "$which GRANT LOG BROKEN for $who: $v")
            // NO SILENT BRANCH. This once swallowed an empty answer as "nothing
            // replicated yet", and an empty answer was exactly what a wrong
            // argument produced — so the check ran every pass, checked nothing,
            // and said nothing.
            else -> Log.w(TAG, "$which chain check for $who said: '$v'")
        }
    }

    companion object {
        const val TAG = "diaswarm"

        /** How long to leave a subject alone between keys-identity offers. */
        private const val HANDOVER_RETRY_MS = 30 * 60 * 1000L

        /** Silence longer than this is a stall, not a slow network. */
        private const val STALE_AFTER = 10 * 60 * 1000L

        /** And never reconnect more often than this, however quiet it gets. */
        private const val MIN_BETWEEN_RESTARTS = 15 * 60 * 1000L
        const val UNIQUE = "diaswarm-sync"

        /**
         * How often to ask while somebody is looking at the screen.
         *
         * A CGM produces a reading every one to five minutes, so asking faster
         * mostly discovers that nothing changed — and still costs a connection.
         */
        const val EVERY_SECONDS = 120L
    }
}

/** The one place that asks for a pass, so there is one policy and not three. */
object Sync {

    private const val NOW = "diaswarm-sync-now"

    /**
     * Ask for a pass now.
     *
     * `KEEP`, so a pass already running is left to finish: it is about to do
     * this work anyway, and cancelling it mid-fetch to start again is how a
     * refresh loop becomes a refresh stall.
     */
    fun now(context: android.content.Context) {
        androidx.work.WorkManager.getInstance(context).enqueueUniqueWork(
            NOW,
            androidx.work.ExistingWorkPolicy.KEEP,
            androidx.work.OneTimeWorkRequestBuilder<SyncWorker>().build()
        )
    }
}
