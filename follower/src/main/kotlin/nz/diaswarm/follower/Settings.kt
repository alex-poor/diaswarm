package nz.diaswarm.follower

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.*
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
    onKeysOnly: () -> Unit,
    keysOnly: Boolean,
    onStayReachable: () -> Unit,
    stayReachable: Boolean,
    dozeExempt: Boolean,
    onFixDoze: () -> Unit,
    onBand: () -> Unit,
    bandLabel: String,
    mmol: Boolean,
    alertOn: (Alerts.Level) -> Boolean,
    alertSubtitle: (Alerts.Level) -> String,
    onToggleAlert: (Alerts.Level) -> Unit,
    notificationsBlocked: Boolean,
    onFixNotifications: () -> Unit,
    dndBypassGranted: Boolean,
    onDndBypass: () -> Unit,
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
                                selected = false,
                                leading = Icons.Outlined.Person
                            ) { onChoose(s) }
                            if (i < followed.lastIndex) RowDivider()
                        }
                    }
                }

                // Built by hand rather than with SettingsGroup so the "blocked"
                // banner can sit between the heading and the switches — a person
                // toggling an alert on should see, right there, that nothing will
                // reach them until notifications are allowed.
                SectionHeading("Alerts")
                if (notificationsBlocked) {
                    Box(Modifier.padding(bottom = AyniSpace.s2)) {
                        StatusBanner(
                            BannerTone.Alert,
                            "Notifications are off — alerts can't reach you. Tap to allow.",
                            Modifier.clickable { onFixNotifications() }
                        )
                    }
                }
                Card(
                    colors = CardDefaults.cardColors(containerColor = AyniColor.Card),
                    shape = RoundedCornerShape(AyniRadius.Card),
                    modifier = Modifier.fillMaxWidth()
                ) {
                    Column {
                        Alerts.Level.entries.forEachIndexed { i, level ->
                            SwitchRow(
                                level.label.replaceFirstChar { it.uppercase() },
                                alertSubtitle(level),
                                checked = alertOn(level),
                                leading = alertIcon(level)
                            ) { onToggleAlert(level) }
                            if (i < Alerts.Level.entries.lastIndex) RowDivider()
                        }
                        // Only urgent lows are ever allowed past Do Not Disturb, and
                        // only when that alert is on — so the choice appears where it
                        // applies and nowhere else.
                        if (alertOn(Alerts.Level.URGENT_LOW)) {
                            RowDivider()
                            ActionRow(
                                "Sound urgent lows in Do Not Disturb",
                                if (dndBypassGranted) "Allowed — an urgent low will sound even when the phone is silenced."
                                else "Off — tap to let only urgent lows through Do Not Disturb.",
                                leading = Icons.Outlined.NotificationsActive
                            ) { onDndBypass() }
                        }
                    }
                }

                SettingsGroup("Add someone") {
                    ActionRow("Scan their invite", leading = Icons.Outlined.QrCodeScanner) { onScan() }
                    RowDivider()
                    // An invite arrives by message as often as by camera, so pasting
                    // is a first-class way in, not a fallback.
                    ActionRow("Paste a code", leading = Icons.Outlined.ContentPaste) { onPaste() }
                    RowDivider()
                    ActionRow("Show my invite", leading = Icons.Outlined.QrCode2) { onShowInvite() }
                }

                SettingsGroup("This phone") {
                    ValueRow("Units", null, if (mmol) "mmol/L" else "mg/dL",
                        leading = Icons.Outlined.Straighten) { onUnits() }
                    RowDivider()
                    SwitchRow(
                        "Keep up to date while asleep",
                        stayAwakeSubtitle(stayReachable, dozeExempt),
                        checked = stayReachable,
                        leading = Icons.Outlined.DarkMode
                    ) { onStayReachable() }
                    // Offered only when the service is on but Android is still holding
                    // it back — the one case the exemption actually helps.
                    if (stayReachable && !dozeExempt) {
                        RowDivider()
                        ActionRow(
                            "Fix background limits",
                            "If it still falls behind overnight, set ayni to Unrestricted — only you can grant it.",
                            leading = Icons.Outlined.BatteryAlert
                        ) { onFixDoze() }
                    }
                    RowDivider()
                    ValueRow("Colour readings against", "Your lines, not theirs", bandLabel,
                        leading = Icons.Outlined.Palette) { onBand() }
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
                            if (advanced) null else "Tap to show",
                            leading = Icons.Outlined.Science
                        ) { advanced = !advanced }
                        if (advanced) {
                            // The "New vault" toggle is gone — the keys vault is
                            // always on now, because turning it off made an invite
                            // the subject could not act on. See [Prefs.keysVault].
                            // Only the read-side cutover switch remains.
                            RowDivider()
                            SwitchRow(
                                "Ignore old vault",
                                "Reads only the new vault, so anything it is missing shows up straight away. Turn back on if the graph looks wrong.",
                                checked = keysOnly,
                                leading = Icons.Outlined.VisibilityOff
                            ) { onKeysOnly() }
                        }
                    }
                }

                Spacer(Modifier.height(AyniSpace.s6))
            }
        }
    }
}

/** The leading icon for an alert level — arrows that point the way the reading went. */
private fun alertIcon(level: Alerts.Level): androidx.compose.ui.graphics.vector.ImageVector = when (level) {
    Alerts.Level.URGENT_LOW -> Icons.Outlined.KeyboardDoubleArrowDown
    Alerts.Level.LOW -> Icons.Outlined.KeyboardArrowDown
    Alerts.Level.HIGH -> Icons.Outlined.KeyboardArrowUp
    Alerts.Level.VERY_HIGH -> Icons.Outlined.KeyboardDoubleArrowUp
}

/** The three-state explanation under "Keep up to date while asleep". */
private fun stayAwakeSubtitle(on: Boolean, dozeExempt: Boolean): String = when {
    !on -> "Off: readings catch up when you open the app. On, ayni keeps fetching, shows a quiet notification, and uses more battery."
    dozeExempt -> "Fetching while the screen is off. Swipe the notification away by turning this off."
    else -> "Fetching while the screen is off, with a quiet notification."
}
