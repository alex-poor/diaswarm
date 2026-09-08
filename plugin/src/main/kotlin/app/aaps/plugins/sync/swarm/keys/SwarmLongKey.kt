package app.aaps.plugins.sync.swarm.keys

import app.aaps.core.keys.interfaces.LongNonPreferenceKey

/**
 * This plugin's own high-water marks.
 *
 * Its OWN, and that is the point: `DataSyncSelector` gives every sync plugin a
 * separate set, so adding a third consumer disturbs neither NSClientV3 nor
 * xDrip. Nothing here may reuse an `xdrip_` key.
 */
enum class SwarmLongKey(
    override val key: String,
    override val defaultValue: Long,
    override val exportable: Boolean = true
) : LongNonPreferenceKey {

    GlucoseValueLastSyncedId("swarm_glucose_value_last_synced_id", 0L),
    BolusLastSyncedId("swarm_bolus_last_synced_id", 0L),
    CarbsLastSyncedId("swarm_carbs_last_synced_id", 0L),
    TemporaryBasalLastSyncedId("swarm_temporary_basal_last_synced_id", 0L),
    ExtendedBolusLastSyncedId("swarm_extended_bolus_last_synced_id", 0L),
    TherapyEventLastSyncedId("swarm_therapy_event_last_synced_id", 0L),
    TemporaryTargetLastSyncedId("swarm_temporary_target_last_synced_id", 0L),

    /** How many genuine post-emit edits have been seen. See spec §7. */
    AmendmentsSeen("swarm_amendments_seen", 0L),
}
