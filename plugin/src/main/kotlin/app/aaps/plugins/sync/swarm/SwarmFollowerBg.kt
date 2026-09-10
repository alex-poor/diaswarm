package app.aaps.plugins.sync.swarm

import android.content.Context
import app.aaps.core.data.model.EPS
import app.aaps.core.data.model.GV
import app.aaps.core.data.model.GlucoseUnit
import app.aaps.core.data.model.ICfg
import app.aaps.core.data.model.SourceSensor
import app.aaps.core.data.model.TrendArrow
import app.aaps.core.data.model.data.Block
import app.aaps.core.data.model.data.TargetBlock
import org.json.JSONObject
import app.aaps.core.data.ue.Sources
import app.aaps.core.interfaces.configuration.Config
import app.aaps.core.interfaces.db.PersistenceLayer
import app.aaps.core.interfaces.logging.AAPSLogger
import app.aaps.core.interfaces.logging.LTag
import app.aaps.core.keys.interfaces.Preferences
import app.aaps.plugins.sync.swarm.keys.SwarmLongKey
import app.aaps.plugins.sync.swarm.keys.SwarmStringKey
import javax.inject.Inject
import javax.inject.Singleton

/**
 * Put a followed person's glucose on this phone's own AAPS graph.
 *
 * WHAT NIGHTSCOUT ACTUALLY DOES, which is the whole target: it shows somebody
 * else's trend line in an app on your phone. Everything else this project has
 * built — sealing, grants, the pool, log sync — moves the readings; this is the
 * twenty lines that make them visible in the place a person already looks. A
 * number in a settings dialog is not a follower app.
 *
 * It does that by writing into AAPS's own `glucoseValues` table, because that
 * is where the graph reads from: `PrepareBgDataWorker` queries the database
 * directly and no BG-source plugin needs to be enabled for a row to be drawn.
 * There is no lighter way in. A second graph drawn by this add-on would be a
 * different screen with different code, and the claim being demonstrated is
 * that the data arrives well enough to drive the real one.
 *
 * ---------------------------------------------------------------------------
 * THE ONLY WRITE THIS PROJECT MAKES INTO THE LOOP APP'S DATABASE, AND WHY IT
 * CANNOT REACH THE LOOP APP
 *
 * Decision D7 says this add-on is read-only with respect to AAPS, and it stays
 * true: the app that drives the pump never runs this code. A followed person's
 * glucose in the database of a phone that doses would be poison — inserting a
 * `GlucoseValue` fires `EventNewBG` from `CompatDBHelper`, `EventNewBG` extends
 * `EventLoop`, and `InvokeLoopWorker` runs the loop on exactly that event. It
 * would not merely draw a wrong line. It would dose on somebody else's blood.
 *
 * So the fence is not a preference and not a runtime check that something else
 * could flip. It is [Config.AAPSCLIENT] — the build flavour:
 *
 *  * `aapsclient` has its own `applicationId` (`info.nightscout.aapsclient`),
 *    so it INSTALLS ALONGSIDE the loop app rather than over it. The follower is
 *    a second app, with its own database. Nothing it writes is visible to the
 *    app holding the pump key, even on the same phone.
 *  * `Config.PUMPDRIVERS` is `full || pumpcontrol`, so this build has no pump
 *    driver compiled into it at all. There is no pump to command.
 *  * `LoopPlugin` is `alwaysEnabled(config.APS)` and `config.APS` is false here.
 *
 * A build flavour cannot be turned on by a setting, restored by a preference
 * import, or reached by a bug in this file. To defeat it you have to install a
 * different APK, which is a decision a person makes with their eyes open.
 *
 * TWO FURTHER BARRIERS, because one fence around this is not enough:
 *
 *  * Every mirrored row is tagged in `ids.nightscoutId` with [MIRROR_TAG] and
 *    the subject it came from, and [DataSyncSelectorSwarmImpl] refuses to
 *    publish a tagged row. Without that, a follower would re-publish the person
 *    they follow as though it were their own glucose, and anyone following the
 *    follower would see the wrong person's blood attributed to them. The tag is
 *    also what makes a mirrored row identifiable in a database dump forever
 *    after — `Sources` is not stored on the row, only written to a log line.
 *  * Nothing is mirrored unless somebody names whose line it is
 *    ([SwarmStringKey.FollowerGraphSubject]), or exactly one person is followed
 *    and so there is nothing to name.
 *
 * WHAT IS STILL TRUE AND UNGUARDED: an AAPS database exported from a follower
 * and imported into a looping phone would carry these rows across. That is an
 * import of a whole foreign database into a medical device, which is already
 * catastrophic for a dozen reasons that have nothing to do with this file; the
 * tag at least means the rows can be found afterwards.
 * ---------------------------------------------------------------------------
 */
@Singleton
class SwarmFollowerBg @Inject constructor(
    private val aapsLogger: AAPSLogger,
    private val preferences: Preferences,
    private val persistenceLayer: PersistenceLayer,
    private val config: Config,
    private val context: Context,
) {

    companion object {

        /**
         * Stamped into `ids.nightscoutId` so a mirrored row is never mistaken
         * for one this phone measured.
         *
         * `nightscoutId` because it is the only free-text field on a
         * `GlucoseValue` that survives `CgmSourceTransaction` — that transaction
         * copies an existing nsId onto an incoming row only when the incoming
         * one is null, so a value set here stays set. `sourceSensor` was the
         * other candidate and is worse: it is half of the transaction's dedupe
         * key, and overwriting it would also throw away which sensor the reading
         * actually came from, which is the one piece of provenance worth having
         * on the graph.
         */
        const val MIRROR_TAG = "swarm-mirror:"

        /**
         * How far back the first mirror reaches.
         *
         * A DAY, NOT A HISTORY. The grant may carry years; the graph shows
         * hours. Pulling everything on first contact would spend minutes of
         * decryption and thousands of indexed inserts to draw a line nobody is
         * looking at, and a follower's first impression would be a frozen app.
         * Whatever else has been granted stays in the vault and is still there.
         */
        const val FIRST_WINDOW_MS = 24 * 60 * 60 * 1000L

        /**
         * Rows per pass, which also bounds the single Java String the readings
         * cross JNI in.
         *
         * A Libre 3 produces about 1,586 readings a day (measured on this
         * subject's own history, not a specification), so this is a bit over a
         * day of catching up per pass and roughly 80 KB of string. A follower
         * further behind than that gets the rest on the next pass two minutes
         * later, which is the right trade: the recent end of the line appears
         * immediately and the tail fills in behind it.
         */
        const val MAX_PER_PASS = 2_000
    }

    /** What one pass did, as a log line, or null when it did nothing at all. */
    fun mirror(): String? {
        // THE FENCE. Not a preference — the build flavour. See the class comment.
        if (!config.AAPSCLIENT) {
            // Silent unless somebody actually asked for it, because every
            // publishing phone runs this code on every pass and a log line per
            // pass saying "no" is how a real warning gets missed.
            if (preferences.get(SwarmStringKey.FollowerGraphSubject).isNotBlank()) {
                aapsLogger.warn(
                    LTag.CORE,
                    "swarm: REFUSING to mirror followed glucose — this is the ${config.FLAVOR} " +
                        "build (${config.APPLICATION_ID}), which can dose. Install the AAPSClient " +
                        "build to follow somebody; it is a separate app and has no pump drivers."
                )
            }
            return null
        }

        val store = SwarmPaths.store(context).absolutePath
        val identity = SwarmPaths.identity(context).absolutePath
        val (subject, purpose) = chosen(store) ?: return null

        val now = System.currentTimeMillis()

        // THE MARK BELONGS TO A SUBJECT, NOT TO THE PHONE. Point the graph at
        // somebody else and the old mark is still sitting at the last person's
        // newest reading, which is very likely ahead of anything the new one
        // has — so the new subject would look like somebody with no data, for
        // ever, with nothing anywhere saying why.
        val whose = subject.hashCode().toLong()
        if (preferences.get(SwarmLongKey.FollowerMirroredSubject) != whose) {
            preferences.put(SwarmLongKey.FollowerMirroredSubject, whose)
            preferences.put(SwarmLongKey.FollowerMirroredThrough, 0L)
            aapsLogger.info(LTag.CORE, "swarm: now graphing ${subject.take(8)} — starting a day back")
        }

        val mark = preferences.get(SwarmLongKey.FollowerMirroredThrough)
        val since = if (mark > 0) mark else now - FIRST_WINDOW_MS

        // THE GRAPH NEEDS THEIR PROFILE BEFORE IT NEEDS THEIR READINGS. Ordered
        // deliberately: with no profile in force AAPS draws an empty chart no
        // matter how much glucose is in the database, so a first pass that
        // inserted 1,631 readings and no profile would look exactly like a
        // first pass that received nothing.
        applyTheirProfile(store, identity, subject, purpose)

        val rows = SwarmNative.netGlucose(store, subject, identity, purpose, since, MAX_PER_PASS)
            .lines()
            .filter { it.isNotBlank() }
        if (rows.isEmpty()) return null

        val tag = MIRROR_TAG + subject.take(16)
        val values = rows.mapNotNull { row -> readingFrom(row, tag) }
        if (values.isEmpty()) {
            // Lines arrived and none of them parsed. That is a spec or encoding
            // fault, not an empty pass, and it must not look like one.
            aapsLogger.warn(LTag.CORE, "swarm: ${rows.size} followed reading(s) and none parsed")
            return null
        }

        val result = persistenceLayer
            .insertCgmSourceData(Sources.NSClient, values, emptyList(), null)
            .blockingGet()

        // ADVANCE ON WHAT WAS OFFERED, NOT ON WHAT WAS INSERTED. A reading this
        // phone already holds comes back as neither inserted nor updated, and a
        // mark that only moved on inserts would re-offer the same readings for
        // ever once the two sides agreed.
        //
        // CLAMPED TO NOW, because one reading with a bad clock on it would
        // otherwise park the mark in the future and every real reading after it
        // would be silently skipped — a follower stuck for ever on the day
        // somebody's phone was set wrong. The reading itself is still stored;
        // it is only disqualified from moving the mark.
        val newest = values.maxOf { it.timestamp }.coerceAtMost(now)
        preferences.put(SwarmLongKey.FollowerMirroredThrough, newest)

        val behind = (System.currentTimeMillis() - newest) / 60_000
        return "swarm: mirrored ${result.inserted.size} new, ${result.updated.size} changed " +
            "of ${values.size} reading(s) from ${subject.take(8)} — newest ${behind}m old" +
            if (rows.size >= MAX_PER_PASS) ", more to come" else ""
    }

    /**
     * Put the followed subject's own profile in force on this phone, once.
     *
     * **WITHOUT THIS THE GRAPH IS BLANK NO MATTER WHAT ARRIVES.** AAPS draws
     * glucose in the units of the profile in force and against its target band,
     * and `OverviewFragment.buildChartData` returns an empty chart the moment
     * `profileFunction.getProfile()` is null. That function reads an
     * `EffectiveProfileSwitch`, which is only ever written when a **pump**
     * accepts a basal profile — and in AAPSCLIENT mode the one fallback reads
     * the profile out of Nightscout's `deviceStatus`. A follower with no
     * Nightscout and no pump therefore never gets one, and every reading it
     * receives is invisible. Measured on a real phone: 1,631 readings in the
     * database, the correct number and delta on screen, and an empty chart.
     *
     * **THEIR PROFILE, NOT A LOCAL ONE**, which is the whole reason `profile`
     * records are in §2 with their blocks normalised to mg/dL. A locally
     * invented profile would draw somebody else's blood against a stranger's
     * targets — a follower glancing at the band would be reading a judgement
     * about the subject that nobody made. Nightscout followers show the
     * subject's profile for the same reason.
     *
     * ONCE, NOT EVERY PASS. An `EffectiveProfileSwitch` is a dated record of
     * "this is what was in force from here"; writing one every two minutes
     * would fabricate a history of profile changes that never happened. This
     * writes only when nothing is in force, or when what the subject publishes
     * no longer matches what is.
     *
     * Failure is not fatal to the pass — a follower with readings and no
     * profile is worth more than a follower with neither, and the log says
     * which happened.
     */
    private fun applyTheirProfile(store: String, identity: String, subject: String, purpose: String) {
        val json = SwarmNative.netProfile(store, subject, identity, purpose)
        if (json.isBlank()) return

        val theirs = try {
            profileFrom(JSONObject(json), subject)
        } catch (e: Exception) {
            aapsLogger.error(LTag.CORE, "swarm: could not read ${subject.take(8)}'s profile", e)
            null
        } ?: return

        val inForce = persistenceLayer.getEffectiveProfileSwitchActiveAt(System.currentTimeMillis())
        // `contentEqualsTo` compares the timestamp too, so it cannot answer
        // "is this the same profile?" — only the blocks can, and they are what
        // the graph is drawn from.
        val same = inForce != null &&
            inForce.basalBlocks == theirs.basalBlocks &&
            inForce.isfBlocks == theirs.isfBlocks &&
            inForce.icBlocks == theirs.icBlocks &&
            inForce.targetBlocks == theirs.targetBlocks
        if (same) return

        persistenceLayer.insertEffectiveProfileSwitch(theirs).blockingGet()
        aapsLogger.info(
            LTag.CORE,
            "swarm: now showing ${subject.take(8)}'s profile \"${theirs.originalProfileName}\" — " +
                "target ${theirs.targetBlocks.firstOrNull()?.lowTarget?.toInt()}" +
                "-${theirs.targetBlocks.firstOrNull()?.highTarget?.toInt()} mg/dL"
        )
    }

    /**
     * A `profile` record as the effective profile switch AAPS draws against.
     *
     * Timestamped NOW, not at the record's own time. This says "from here on,
     * this is what the graph means"; back-dating it to when the subject
     * published would claim this phone had been showing their profile for
     * however long ago that was, which it had not.
     *
     * Blocks arrive already in mg/dL (§2), so nothing is converted here — and
     * converting would be the bug, because the subject's phone may be set to
     * mmol/L and the same numbers would then be eighteen times wrong.
     *
     * Returns null rather than half a profile if the blocks are missing: AAPS
     * requires each set to cover a whole day, and a partial profile is worse
     * than none.
     */
    private fun profileFrom(o: JSONObject, subject: String): EPS? {
        fun blocks(key: String): List<Block> {
            val a = o.optJSONArray(key) ?: return emptyList()
            return (0 until a.length()).map {
                val b = a.getJSONObject(it)
                Block(b.getLong("duration"), b.getDouble("amount"))
            }
        }

        val basal = blocks("basal")
        val isf = blocks("isf")
        val ic = blocks("ic")
        val targets = o.optJSONArray("target")?.let { a ->
            (0 until a.length()).map {
                val b = a.getJSONObject(it)
                TargetBlock(b.getLong("duration"), b.getDouble("lowTarget"), b.getDouble("highTarget"))
            }
        } ?: emptyList()
        if (basal.isEmpty() || isf.isEmpty() || ic.isEmpty() || targets.isEmpty()) return null

        val name = o.optString("name").ifBlank { "swarm" }
        return EPS(
            timestamp = System.currentTimeMillis(),
            basalBlocks = basal,
            isfBlocks = isf,
            icBlocks = ic,
            targetBlocks = targets,
            // §2 normalises to mg/dL on the way out, whatever the subject's
            // phone displays. Saying anything else here would rescale the axis.
            glucoseUnit = GlucoseUnit.MGDL,
            // NAMED AFTER WHO IT BELONGS TO. This name appears on the follower's
            // screen where their own profile name normally sits, and a follower
            // who reads "LocalProfile1" there will believe it is theirs.
            originalProfileName = "$name (${subject.take(8)})",
            originalCustomizedName = "$name (${subject.take(8)})",
            originalTimeshift = o.optLong("shift", 0L),
            originalPercentage = o.optInt("pct", 100),
            // Zero: no end. A followed profile stays in force until they change
            // it, and a duration would silently expire the graph.
            originalDuration = 0L,
            originalEnd = 0L,
            // The insulin model is the loop's business, not the graph's, and it
            // is not in §2's vocabulary. These are AAPS's own defaults for a
            // rapid-acting profile and nothing on a follower reads them.
            iCfg = ICfg("Rapid-Acting Oref", 5 * 60 * 60 * 1000L, 75 * 60 * 1000L)
        )
    }

    /**
     * Whose glucose goes on the graph: the named subject, or the only one.
     *
     * A follower who has scanned one person's code should not then have to type
     * that person's key into a settings box to see them — the ambiguity the
     * preference exists to resolve does not exist yet. With several followed it
     * does, and picking one would be picking a person at random.
     */
    private fun chosen(store: String): Pair<String, String>? {
        val follows = SwarmNative.netFollowing(store).lines()
            .filter { it.isNotBlank() }
            .map { it.split('\t') }
            // Never reached means there is no replica on disk to open.
            .filter { it.getOrElse(2) { "0" } == "1" }
            .map { it.getOrElse(0) { "" } to it.getOrElse(1) { "follow" } }
            .filter { it.first.isNotEmpty() }

        val named = preferences.get(SwarmStringKey.FollowerGraphSubject).trim()
        return when {
            named.isNotEmpty() -> follows.firstOrNull { it.first.startsWith(named, ignoreCase = true) }
                ?: null.also {
                    aapsLogger.warn(LTag.CORE, "swarm: no followed subject starts with '$named'")
                }

            follows.size == 1  -> follows.first()

            follows.size > 1   -> null.also {
                aapsLogger.debug(
                    LTag.CORE,
                    "swarm: following ${follows.size} people and none chosen for the graph — " +
                        "set swarm_follower_graph_subject"
                )
            }

            else               -> null
        }
    }

    /**
     * One `millis<TAB>mgdl<TAB>trend<TAB>src` line as a glucose value.
     *
     * `trend` and `src` are the enum NAMES [SwarmRecords] wrote, resolved by
     * name rather than by `fromString` — `fromString` matches display text
     * ("Libre3"), not the constant, and would silently turn every reading into
     * UNKNOWN. Anything this build does not recognise becomes UNKNOWN/NONE
     * rather than being dropped: a reading whose sensor model this app has
     * never heard of is still that person's blood glucose.
     */
    private fun readingFrom(row: String, tag: String): GV? {
        val f = row.split('\t')
        val t = f.getOrNull(0)?.toLongOrNull() ?: return null
        val mgdl = f.getOrNull(1)?.toDoubleOrNull() ?: return null
        if (t <= 0 || mgdl <= 0.0) return null
        return GV(
            timestamp = t,
            value = mgdl,
            raw = 0.0,
            noise = null,
            trendArrow = TrendArrow.entries.firstOrNull { it.name == f.getOrNull(2) } ?: TrendArrow.NONE,
            sourceSensor = SourceSensor.entries.firstOrNull { it.name == f.getOrNull(3) } ?: SourceSensor.UNKNOWN,
            ids = app.aaps.core.data.model.IDs(nightscoutId = "$tag:$t")
        )
    }
}
