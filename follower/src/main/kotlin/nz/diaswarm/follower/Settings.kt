package nz.diaswarm.follower

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp

/**
 * The whole menu, as a screen rather than an AlertDialog.
 *
 * It used to be one dialog whose `title` was "People you follow" and whose body
 * held every app-wide setting, so units and the vault toggles read as children
 * of whoever was listed above. Now it is a full screen with grouped sections,
 * real switches, and the experimental vault options folded away under Advanced
 * where they cannot be flipped by accident. System back closes it.
 *
 * Same inputs as the old sheet — the behaviour of each control is unchanged,
 * only its presentation.
 */
@Composable
fun SettingsScreen(
    followed: List<Follower.Subject>,
    onScan: () -> Unit,
    onPaste: () -> Unit,
    onShowInvite: () -> Unit,
    onChoose: (Follower.Subject) -> Unit,
    onUnits: () -> Unit,
    onKeysVault: () -> Unit,
    keysVault: Boolean,
    onKeysOnly: () -> Unit,
    keysOnly: Boolean,
    onStayReachable: () -> Unit,
    stayReachable: Boolean,
    dozeExempt: Boolean,
    onFixDoze: () -> Unit,
    onBand: () -> Unit,
    bandLabel: String,
    mmol: Boolean,
    onClose: () -> Unit
) {
    BackHandler { onClose() }
    Surface(Modifier.fillMaxSize(), color = AyniColor.Bg) {
        Column(Modifier.fillMaxSize().windowInsetsPadding(WindowInsets.statusBars)) {

            // Top bar: a back chevron that closes, and the title.
            Row(
                Modifier.fillMaxWidth().padding(horizontal = AyniSpace.s2, vertical = AyniSpace.s2),
                verticalAlignment = Alignment.CenterVertically
            ) {
                Box(Modifier.clip(CircleShape).clickable { onClose() }.padding(AyniSpace.s2)) {
                    Text("‹", color = AyniColor.Text1, fontSize = 26.sp)
                }
                Spacer(Modifier.width(AyniSpace.s1))
                Text("Settings", color = AyniColor.Text1, fontSize = AyniType.Title, fontWeight = FontWeight.SemiBold)
            }

            Column(
                Modifier.weight(1f).verticalScroll(rememberScrollState()).padding(horizontal = AyniSpace.s4),
                verticalArrangement = Arrangement.spacedBy(2.dp)
            ) {
                SettingsGroup("People you follow") {
                    if (followed.isEmpty()) {
                        Box(Modifier.padding(horizontal = AyniSpace.s4, vertical = 14.dp)) {
                            Text("Nobody yet.", color = AyniColor.Text2, fontSize = AyniType.Body)
                        }
                    } else {
                        followed.forEachIndexed { i, s ->
                            // "Held but unreadable" and "nothing arrived" are different
                            // problems and must not look alike.
                            SubjectRow(
                                s.short,
                                if (s.reached) "holding their history" else "nothing has arrived yet",
                                selected = false
                            ) { onChoose(s) }
                            if (i < followed.lastIndex) RowDivider()
                        }
                    }
                }

                SettingsGroup("Add someone") {
                    ActionRow("Scan their invite") { onScan() }
                    RowDivider()
                    // An invite arrives by message as often as by camera, so pasting
                    // is a first-class way in, not a fallback.
                    ActionRow("Paste a code") { onPaste() }
                    RowDivider()
                    ActionRow("Show my invite") { onShowInvite() }
                }

                SettingsGroup("This phone") {
                    ValueRow("Units", null, if (mmol) "mmol/L" else "mg/dL") { onUnits() }
                    RowDivider()
                    SwitchRow(
                        "Keep up to date while asleep",
                        stayAwakeSubtitle(stayReachable, dozeExempt),
                        checked = stayReachable
                    ) { onStayReachable() }
                    // Offered only when the service is on but Android is still holding
                    // it back — the one case the exemption actually helps.
                    if (stayReachable && !dozeExempt) {
                        RowDivider()
                        ActionRow(
                            "Fix background limits",
                            "If it still falls behind overnight, set ayni to Unrestricted — only you can grant it."
                        ) { onFixDoze() }
                    }
                    RowDivider()
                    ValueRow("Colour readings against", "Your lines, not theirs", bandLabel) { onBand() }
                }

                // The experimental vault options, folded away. A phone reading
                // neither vault shows a blank screen, so these are not something to
                // flip past on the way to Units.
                var advanced by remember { mutableStateOf(false) }
                SectionHeading("Advanced")
                Card(
                    colors = CardDefaults.cardColors(containerColor = AyniColor.Card),
                    shape = RoundedCornerShape(AyniRadius.Card),
                    modifier = Modifier.fillMaxWidth()
                ) {
                    Column {
                        ActionRow(
                            "Experimental vault options",
                            if (advanced) null else "Tap to show"
                        ) { advanced = !advanced }
                        if (advanced) {
                            RowDivider()
                            SwitchRow(
                                "New vault",
                                "Your invite changes form, so the person you follow needs a recent app for it to work.",
                                checked = keysVault
                            ) { onKeysVault() }
                            if (keysVault) {
                                RowDivider()
                                SwitchRow(
                                    "Ignore old vault",
                                    "Reads only the new vault, so anything it is missing shows up straight away. Turn back on if the graph looks wrong.",
                                    checked = keysOnly
                                ) { onKeysOnly() }
                            }
                        }
                    }
                }

                Spacer(Modifier.height(AyniSpace.s6))
            }
        }
    }
}

/** The three-state explanation under "Keep up to date while asleep". */
private fun stayAwakeSubtitle(on: Boolean, dozeExempt: Boolean): String = when {
    !on -> "Off: readings catch up when you open the app. On, ayni keeps fetching, shows a quiet notification, and uses more battery."
    dozeExempt -> "Fetching while the screen is off. Swipe the notification away by turning this off."
    else -> "Fetching while the screen is off, with a quiet notification."
}
