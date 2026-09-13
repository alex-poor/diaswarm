package nz.diaswarm.follower

import androidx.compose.foundation.Image
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.*
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp

private val Text1 = Color(0xFFE8EAED)
private val Text2 = Color(0xFF9AA3B0)

/**
 * The code somebody else scans to start following you — or, in the one-scan
 * flow, the code they scan so THEY can share with YOU and hand their invite
 * back in the same movement.
 *
 * Opening this opened the offer window ([Endpoint.expectOffer]); the invite is
 * shown as text as well, because a camera is not always in the room and an
 * invite is short enough to send in a message.
 */
@Composable
fun InviteDialog(invite: String, onClose: () -> Unit) {
    val context = LocalContext.current
    AlertDialog(
        onDismissRequest = onClose,
        confirmButton = {
            TextButton(onClick = {
                val cb = context.getSystemService(android.content.ClipboardManager::class.java)
                cb?.setPrimaryClip(android.content.ClipData.newPlainText("diaswarm invite", invite))
                onClose()
            }) { Text("Copy") }
        },
        dismissButton = { TextButton(onClick = onClose) { Text("Close") } },
        title = { Text("Your invite") },
        text = {
            Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
                if (invite.isEmpty()) {
                    Text("Not ready yet — this phone is still joining the network.", color = Text2)
                } else {
                    Qr.bitmap(context, invite)?.let {
                        Image(it.asImageBitmap(), contentDescription = "invite code", modifier = Modifier.fillMaxWidth())
                    }
                    Text(
                        "It is not a secret: it holds a public key and an address. Anyone who has " +
                            "it can download your ciphertext and read none of it.",
                        color = Text2, fontSize = 13.sp
                    )
                    Text(invite, color = Text2, fontSize = 10.sp)
                }
            }
        }
    )
}

/** Who this phone follows, and the handful of controls that exist. */
@Composable
fun PeopleSheet(
    followed: List<Follower.Subject>,
    onScan: () -> Unit,
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
    AlertDialog(
        onDismissRequest = onClose,
        confirmButton = { TextButton(onClick = onClose) { Text("Close") } },
        title = { Text("People you follow") },
        text = {
            Column(verticalArrangement = Arrangement.spacedBy(10.dp)) {
                if (followed.isEmpty()) {
                    Text("Nobody yet.", color = Text2)
                } else {
                    followed.forEach { s ->
                        Column(
                            Modifier.fillMaxWidth().clip(RoundedCornerShape(8.dp))
                                .clickable { onChoose(s) }.padding(vertical = 8.dp)
                        ) {
                            Text(s.short, color = Text1, fontSize = 15.sp)
                            // "Held but unreadable" and "nothing arrived" are
                            // different problems and must not look alike.
                            Text(
                                if (s.reached) "holding their history" else "nothing has arrived yet",
                                color = Text2, fontSize = 12.sp
                            )
                        }
                    }
                }
                Divider(color = Color(0xFF2A3140))
                Text("Scan an invite", color = Text1, fontSize = 15.sp,
                    modifier = Modifier.fillMaxWidth().clickable { onScan() }.padding(vertical = 8.dp))
                Text("Show my invite", color = Text1, fontSize = 15.sp,
                    modifier = Modifier.fillMaxWidth().clickable { onShowInvite() }.padding(vertical = 8.dp))
                Text("Units: ${if (mmol) "mmol/L" else "mg/dL"}", color = Text1, fontSize = 15.sp,
                    modifier = Modifier.fillMaxWidth().clickable { onUnits() }.padding(vertical = 8.dp))
                // BEFORE THE EXPERIMENTAL ROWS, because this one is about
                // battery and freshness and applies to everybody.
                Column(
                    Modifier.fillMaxWidth().clip(RoundedCornerShape(8.dp))
                        .clickable { if (stayReachable && !dozeExempt) onFixDoze() else onStayReachable() }
                        .padding(vertical = 8.dp)
                ) {
                    Text(
                        "Keep up to date while asleep: ${if (stayReachable) "on" else "off"}",
                        color = Text1,
                        fontSize = 15.sp
                    )
                    // **THREE THINGS, AND ALL THREE SAID BEFORE THE TAP.** What
                    // it does, what it costs, and — when it is on but only half
                    // done — what is still missing and where to fix it.
                    // Measured, not guessed: two hours unplugged produced forty
                    // minutes with no fetch at all.
                    Text(
                        if (!stayReachable)
                            "Off: readings stop while the screen is off and catch up when you " +
                                "open the app — measured at 40 minutes behind after two hours " +
                                "face down. On, ayni keeps fetching, shows a permanent " +
                                "notification, and uses more battery."
                        else if (dozeExempt)
                            "Fetching while the screen is off. There is a permanent " +
                                "notification while this is on; swipe it away by turning this off."
                        else
                            "Fetching while the screen is off, with a permanent notification. " +
                                "If it still falls behind overnight, tap here and set ayni to " +
                                "Unrestricted — that is Android's last word on it, and only " +
                                "you can grant it.",
                        // NOT A WARNING ANY MORE. This said "Android will still
                        // pause ayni", and then measurement disagreed: with the
                        // service running, `dumpsys netpolicy` reports
                        // `effective=NONE` in deep doze and the fetches keep
                        // landing on their two-minute cadence. The exemption is
                        // offered as a remedy if it is ever needed, rather than
                        // demanded on a theory — asking for a battery
                        // exemption nobody needs is how apps train people to
                        // grant them without reading.
                        color = Text2,
                        fontSize = 12.sp
                    )
                }
                Column(
                    Modifier.fillMaxWidth().clip(RoundedCornerShape(8.dp))
                        .clickable { onKeysVault() }.padding(vertical = 8.dp)
                ) {
                    Text(
                        "New vault: ${if (keysVault) "on" else "off"}",
                        color = Text1,
                        fontSize = 15.sp
                    )
                    // SAYS WHAT IT COSTS, because the cost lands on somebody
                    // else. Turning this on makes your invite readable only by
                    // apps that understand it — so if the person you follow has
                    // not updated, sharing stops working rather than degrading.
                    Text(
                        "Experimental. Your invite changes form, so the person you " +
                            "follow needs a recent app for it to work.",
                        color = Text2,
                        fontSize = 12.sp
                    )
                }
                // ONLY OFFERED WHILE THE NEW VAULT IS ON, because a phone
                // reading neither source shows a blank screen, and a blank
                // screen is not a test of anything.
                if (keysVault) {
                    Column(
                        Modifier.fillMaxWidth().clip(RoundedCornerShape(8.dp))
                            .clickable { onKeysOnly() }.padding(vertical = 8.dp)
                    ) {
                        Text(
                            "Old vault: ${if (keysOnly) "ignored" else "still read"}",
                            color = Text1,
                            fontSize = 15.sp
                        )
                        // THE ONLY WAY TO ACTUALLY TEST THE CUTOVER. Both
                        // vaults are read and merged, which is right during a
                        // migration and is also why "the new one can stand
                        // alone" has never been checked — the old one has been
                        // covering for it at every step. Turn it off and
                        // anything missing shows up now, not on the day the old
                        // vault is retired.
                        Text(
                            "Ignores the old vault, so anything the new one is " +
                                "missing shows up straight away. Turn it back on if " +
                                "the graph looks wrong.",
                            color = Text2,
                            fontSize = 12.sp
                        )
                    }
                }
                Column(
                    Modifier.fillMaxWidth().clip(RoundedCornerShape(8.dp))
                        .clickable { onBand() }.padding(vertical = 8.dp)
                ) {
                    Text("Colour readings against: $bandLabel", color = Text1, fontSize = 15.sp)
                    // SAYS WHOSE THRESHOLD IT IS, every time it is shown. The
                    // person you follow may loop to a single number and publish
                    // no range at all; this is the viewer's choice of where to
                    // draw high and low, and attributing it to them would be
                    // putting a clinical threshold in their mouth.
                    Text(
                        "Your setting, not theirs — they publish a target, not a range. " +
                            "The consensus bands, or no colouring at all.",
                        color = Text2, fontSize = 12.sp
                    )
                }
            }
        }
    )
}
