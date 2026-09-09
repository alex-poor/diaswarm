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
    ProfileSwitchLastSyncedId("swarm_profile_switch_last_synced_id", 0L),

    /** How many genuine post-emit edits have been seen. See spec §7. */
    AmendmentsSeen("swarm_amendments_seen", 0L),
}

/**
 * String preferences, and these ones are meant to be SEEN.
 *
 * `StringPreferenceKey`, not `StringNonPreferenceKey`. The distinction is not
 * cosmetic: a key that is not registered is invisible to `Preferences.get(String)`,
 * is silently dropped from settings export, and — in this fork, whose preference
 * list is rendered by Compose off the typed key — falls through to a plain
 * click-through row that does nothing. Granting stayed an adb operation partly
 * because of this one word.
 */
enum class SwarmStringKey(
    override val key: String,
    override val defaultValue: String,
    override val defaultedBySM: Boolean = false,
    override val showInApsMode: Boolean = true,
    override val showInNsClientMode: Boolean = true,
    override val showInPumpControlMode: Boolean = true,
    override val dependency: app.aaps.core.keys.interfaces.BooleanPreferenceKey? = null,
    override val negativeDependency: app.aaps.core.keys.interfaces.BooleanPreferenceKey? = null,
    override val hideParentScreenIfHidden: Boolean = false,
    override val isPassword: Boolean = false,
    override val isPin: Boolean = false,
    override val exportable: Boolean = true
) : app.aaps.core.keys.interfaces.StringPreferenceKey {

    /**
     * A reader to grant, as 64 hex characters, acted on and cleared next pass.
     *
     * A PREFERENCE RATHER THAN A FILE DROP, because the mechanism is the part
     * that lasts: this is where a settings screen will write, and until there is
     * one it can be set with adb. A file watched in app storage would have been
     * quicker and would still be here in a year.
     */
    GrantReader("swarm_grant_reader", ""),

    /** A reader to withdraw from. Same shape, same lifecycle. */
    RevokeReader("swarm_revoke_reader", ""),

    /**
     * Identity for the "show my invite" button, which stores nothing.
     *
     * `AdaptiveClickPreference` requires a `StringPreferenceKey` even when the
     * row is a button, so an empty-defaulted key is how AAPS gives one an
     * identity — `StringKey.OverviewCopySettingsFromNs` is the same trick.
     * Not exportable: there is no value to export.
     */
    ShowInvite("swarm_show_invite", "", exportable = false),

    /** Same, for the list of who can currently read. */
    ShowReaders("swarm_show_readers", "", exportable = false),
}
