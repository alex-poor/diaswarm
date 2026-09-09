package app.aaps.plugins.sync.swarm

import app.aaps.core.data.model.BS
import app.aaps.core.data.model.CA
import app.aaps.core.data.model.EB
import app.aaps.core.data.model.GV
import app.aaps.core.data.model.PS
import app.aaps.core.data.model.TB
import app.aaps.core.data.model.TE
import app.aaps.core.data.model.TT
import app.aaps.core.data.model.data.Block
import org.json.JSONArray
import org.json.JSONObject

/**
 * AAPS domain objects to `spec/records.md` v2 records.
 *
 * DELIBERATELY RAW. Values cross the JNI boundary unrounded and the native side
 * applies the precision table, because rounding is part of the canonical form:
 * two emitters that round differently produce different bytes for the same
 * reading, and nothing notices until two peers compare streams. The same
 * argument that put the encoding in one crate puts rounding there.
 *
 * DURATIONS ARE MILLISECONDS. AAPS stores them that way and spec v2 says so —
 * v1 said minutes, which would have read a 15-minute temp basal as 625 days.
 * Pass `duration` straight through and do not "helpfully" convert.
 */
object SwarmRecords {

    /**
     * Records the vocabulary covers. Anything else is dropped rather than
     * guessed at: §2 is a closed set, and a kind invented at the emitter is a
     * kind no consumer can be expected to know.
     *
     * Not covered on purpose: `DeviceStatus` and `ProfileStore` are the loop
     * telemetry §4 excludes; `Food`, `BolusCalculatorResult`,
     * `EffectiveProfileSwitch` and `RunningMode` are outside v2's vocabulary and
     * need a spec change before they can be emitted, not a mapping here.
     */
    fun from(value: Any): String? = when (value) {
        is GV -> obj(value.timestamp, "cgm") {
            put("mgdl", value.value)
            put("trend", value.trendArrow.name)
            put("src", value.sourceSensor.name)
        }

        is BS -> obj(value.timestamp, "bolus") {
            put("u", value.amount)
            put("type", value.type.name)
            put("basal", value.isBasalInsulin)
        }

        is CA -> obj(value.timestamp, "carb") {
            put("g", value.amount)
            put("dur", value.duration)
        }

        is TB -> obj(value.timestamp, "tbr") {
            put("rate", value.rate)
            put("abs", value.isAbsolute)
            put("dur", value.duration)
            put("type", value.type.name)
        }

        is EB -> obj(value.timestamp, "extbolus") {
            put("u", value.amount)
            put("dur", value.duration)
        }

        is TE -> obj(value.timestamp, "event") {
            put("type", value.type.name)
            put("dur", value.duration)
            // The field most likely to name a third party (§2). Carried because
            // a site change or an illness note is often the only explanation for
            // a week of odd data — and the first thing a narrowed grant should
            // drop.
            value.note?.takeIf { it.isNotBlank() }?.let { put("note", it) }
            value.glucose?.let { put("mgdl", it) }
        }

        is TT -> obj(value.timestamp, "target") {
            put("lo", value.lowTarget)
            put("hi", value.highTarget)
            put("dur", value.duration)
            put("why", value.reason.name)
        }

        // The blocks ARE the profile. Without basal rates, ISF, IC and targets
        // by time of day a consumer cannot say what the loop was trying to do,
        // which makes every insulin record beside them uninterpretable (§2).
        //
        // `unit` is passed through RAW and the native side converts and removes
        // it. AAPS stores these blocks in whichever unit the user set while
        // glucose values and temporary targets are always mg/dL, so an
        // un-normalised profile puts the same quantity in one stream a factor of
        // eighteen apart. Doing the conversion here would put that rule in two
        // places, and a rule about units in two places is a rule that will
        // disagree with itself.
        is PS -> obj(value.timestamp, "profile") {
            put("name", value.profileName)
            put("pct", value.percentage)
            put("shift", value.timeshift)
            put("dur", value.duration)
            put("unit", value.glucoseUnit.name)
            put("basal", blocks(value.basalBlocks) { JSONObject().put("amount", it.amount) })
            put("isf", blocks(value.isfBlocks) { JSONObject().put("amount", it.amount) })
            put("ic", blocks(value.icBlocks) { JSONObject().put("amount", it.amount) })
            put("target", JSONArray().apply {
                value.targetBlocks.forEach {
                    put(
                        JSONObject()
                            .put("duration", it.duration)
                            .put("lowTarget", it.lowTarget)
                            .put("highTarget", it.highTarget)
                    )
                }
            })
        }

        else -> null
    }

    /** Blocks as `[{duration, …}]`, in the order AAPS holds them. */
    private inline fun blocks(list: List<Block>, field: (Block) -> JSONObject): JSONArray =
        JSONArray().apply {
            list.forEach { put(field(it).put("duration", it.duration)) }
        }

    /**
     * A record is `t` and `k` plus whatever the device reported.
     *
     * A field the device did not report must be ABSENT — not null, not zero.
     * §1 makes that distinction load-bearing, and `JSONObject.put` with a null
     * would write a JSON null, so callers use `?.let { put(...) }` above.
     */
    private inline fun obj(t: Long, kind: String, fill: JSONObject.() -> Unit): String =
        JSONObject().apply {
            put("t", t)
            put("k", kind)
            fill()
        }.toString()
}
