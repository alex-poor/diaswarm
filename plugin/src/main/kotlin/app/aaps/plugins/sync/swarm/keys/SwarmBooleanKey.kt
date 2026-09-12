package app.aaps.plugins.sync.swarm.keys

import app.aaps.core.keys.interfaces.BooleanPreferenceKey

/**
 * Switches, and at present only one.
 *
 * SHADOW MODE EXISTS BECAUSE THE ALTERNATIVE IS A CUTOVER. `diaswarm-keys`
 * replaces the hand-composed sealing construction with p2panda's key layer
 * (D26). It agrees with the old vault record-for-record over 74 days of this
 * subject's real history, and it runs on this phone. Neither of those is the
 * same as having run inside AAPS, on a device driving a pump, for a week.
 *
 * So the first thing it does on a phone is nothing anybody depends on: seal the
 * same records into the new vault, **read them back off disk**, and log whether
 * every one survived. The old vault stays authoritative, every screen keeps
 * reading from it, and if the new one throws or loses a record the only
 * consequence is a log line.
 *
 * **THE READ-BACK IS THE POINT, AND FOR A LONG TIME IT WAS NOT HAPPENING.**
 * This comment said "log whether they agree" and the code added up how many
 * records the shadow vault reported sealing. Nothing was compared with
 * anything, so a shadow vault silently keeping four days out of five read
 * exactly like one working perfectly. Asking what it would have to compare is
 * what found `Vault::seal` replacing a day instead of appending to it.
 *
 * And "agree" means the new vault returns what it was given, not that the two
 * vaults match. The old vault is the thing being replaced and is itself
 * fallible; two vaults can agree by losing the same record.
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
     * Also seal into the `diaswarm-keys` vault, read it back, and report.
     *
     * **Off, and it changes nothing while it is off.** On, it costs a second
     * seal of the same records plus a read of the epoch it just wrote, and
     * writes one line per pass saying whether every record survived —
     * `shadow agrees` or `shadow DISAGREES` with the counts behind it.
     */
    ShadowSpacesVault("swarm_shadow_spaces_vault", false),
}
