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

    /**
     * The newest five-minute CGM bucket already published (spec §3.3).
     *
     * **-1, NOT 0.** Zero is a real bucket — five minutes past midnight on 1
     * January 1970 — so a default of 0 would mean "everything since 1970 is
     * already published" and thin every reading the phone ever sees. The one
     * preference here whose default is not zero, for that reason.
     *
     * Carried across passes because live readings arrive one per minute in
     * separate passes: an emitter that starts fresh each time never sees two
     * readings from one bucket together and thins nothing.
     */
    CgmBucketHighWater("swarm_cgm_bucket_high_water", -1L),

    /**
     * Whether the shadow vault has been filled since it was last switched on.
     *
     * **BECAUSE A BUTTON IS A WORSE MECHANISM THAN A CONSEQUENCE.** Turning
     * shadow mode on and having it hold only what happened afterwards is not
     * useful: the whole point is to compare it against a real history. So
     * switching it on now backfills, once, and this remembers that it did.
     *
     * It also removes a dependency on the re-drain button, which was tapped
     * three times and never once reached its handler.
     */
    ShadowFilled("swarm_shadow_filled", 0L),

    /**
     * The newest followed reading already written into this phone's own database.
     *
     * A HIGH-WATER MARK, NOT A CACHE. `CgmSourceTransaction` is idempotent on
     * `(timestamp, sourceSensor)`, so re-offering a reading is harmless — but a
     * Libre 3 produces about 1,586 readings a day, and re-offering a day of them
     * every two minutes is 1,586 indexed lookups for nothing. This is what keeps
     * a steady-state pass at one or two rows.
     *
     * Zero means "nothing mirrored yet", and [SwarmFollowerBg] starts from a
     * bounded window rather than the subject's whole history: the point is a
     * graph you can read, not a backfill of somebody's year.
     */
    FollowerMirroredThrough("swarm_follower_mirrored_through", 0L),

    /**
     * Which subject [FollowerMirroredThrough] is a mark for, as `hashCode`.
     *
     * WITHOUT THIS THE MARK IS A TRAP. Change whose line is on the graph and
     * the mark is still sitting at the old person's newest reading — which is
     * almost certainly in the future relative to anything the new person's
     * backfill would offer, so the new subject would appear to have no data at
     * all, for ever, and nothing would say why. Cheaper to notice than to
     * diagnose.
     *
     * `String.hashCode` because the mark needs a companion that is invisible on
     * the settings screen, and the long keys are the invisible ones. It is
     * specified by the Java language and so stable across devices and versions;
     * a collision between two subjects the same phone follows costs one
     * skipped backfill and no wrong data.
     */
    FollowerMirroredSubject("swarm_follower_mirrored_subject", 0L),
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
     * Somebody's invite text, followed and cleared on the next pass.
     *
     * **BECAUSE A CAMERA IS NOT ALWAYS IN THE ROOM.** Scanning is the good path
     * when two people are together, and it is the only path there was — which
     * quietly meant that following somebody at a distance was impossible, and
     * that no part of following could be exercised without two phones and a
     * pair of hands. An invite is a short piece of text; it can be sent in a
     * message, and the *Show my invite* dialog already offers to copy it.
     *
     * It carries no authority. An invite says "here is my key and where to
     * reach me"; holding one lets you keep somebody's ciphertext and nothing
     * else. Reading still requires them to grant you, from their phone.
     */
    FollowInvite("swarm_follow_invite", ""),

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

    /** Same, for the camera. */
    ScanCode("swarm_scan_code", "", exportable = false),

    /** Same, for the list of people this phone follows. */
    ShowFollowing("swarm_show_following", "", exportable = false),

    /**
     * Same, for "re-read everything from the database".
     *
     * Exists because the high-water marks are `LongNonPreferenceKey`s and so
     * appear on no screen: without a button there is no way to ask for a
     * re-drain except editing the preferences file as root, which is not
     * something to ask of anybody.
     */
    Resync("swarm_resync", "", exportable = false),

    /**
     * Whose glucose goes on THIS phone's graph, as the subject's key (a prefix
     * is enough), or blank.
     *
     * **THE ONLY WRITE THIS PROJECT MAKES INTO AAPS'S OWN DATABASE**, and it is
     * fenced twice over. [SwarmFollowerBg] refuses unless the build is an
     * AAPSClient flavour — a different `applicationId`, no pump drivers compiled
     * in, and nothing a preference can switch on — so the readings can never
     * land in the app that drives the pump. This key then decides which of
     * possibly several followed people is the one being watched, because AAPS
     * has exactly one glucose series and somebody has to say whose it is.
     *
     * Blank with exactly one subject followed means that subject: there is no
     * ambiguity to resolve and making a follower type a key to see the graph
     * they just scanned a code for is ceremony, not safety. Blank with several
     * followed means nothing is mirrored, because guessing would be picking a
     * person at random.
     */
    FollowerGraphSubject("swarm_follower_graph_subject", ""),
}
