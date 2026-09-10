package app.aaps.plugins.sync.swarm.keys

import app.aaps.core.keys.interfaces.BooleanPreferenceKey

/**
 * Switches, and at present only one.
 *
 * SHADOW MODE EXISTS BECAUSE THE ALTERNATIVE IS A CUTOVER. `diaswarm-spaces`
 * replaces the hand-composed sealing construction with p2panda's key layer
 * (D20) and log sync (D21). It agrees with the old vault record-for-record over
 * 74 days of this subject's real history, and it runs on this phone. Neither of
 * those is the same as having run inside AAPS, on a device driving a pump, for
 * a week.
 *
 * So the first thing it does on a phone is nothing anybody depends on: seal the
 * same records into both vaults and log whether they agree. The old vault stays
 * authoritative, every screen keeps reading from it, and if the new one throws
 * or disagrees the only consequence is a log line.
 */
enum class SwarmBooleanKey(
    override val key: String,
    override val defaultValue: Boolean,
    override val calculatedDefaultValue: Boolean = false,
    override val engineeringModeOnly: Boolean = false,
    override val defaultedBySM: Boolean = false,
    override val showInApsMode: Boolean = true,
    override val showInNsClientMode: Boolean = true,
    override val showInPumpControlMode: Boolean = true,
    override val dependency: BooleanPreferenceKey? = null,
    override val negativeDependency: BooleanPreferenceKey? = null,
    override val hideParentScreenIfHidden: Boolean = false,
    override val exportable: Boolean = true
) : BooleanPreferenceKey {

    /**
     * Also seal into the p2panda-spaces vault, and report whether it agrees.
     *
     * **Off, and it changes nothing while it is off.** On, it costs a second
     * seal of the same records — measured at 31 ms for five days on this
     * hardware — and writes one line per pass saying what each vault holds.
     */
    ShadowSpacesVault("swarm_shadow_spaces_vault", false),
}
