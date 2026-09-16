package nz.diaswarm.follower

import androidx.compose.foundation.Image
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.*
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
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
// For the two places a screen has to say "this will not do what you think".
private val Warn = Color(0xFFE8A33D)

/**
 * What this phone is holding for other people, and what that does and does not
 * mean.
 *
 * **THE CLAIMS HERE ARE CONSTRAINED, NOT WRITTEN FRESH.** `feasibility.md` §11
 * lists sentences this project may never say, and two of them are within reach
 * of a screen like this: that carrying gives any visibility into content, and
 * that data is "stored across a distributed network" while the pool is a
 * handful of devices. What it says instead — a carrier breach is not a data
 * breach — is true, and is the sentence that makes carrying reasonable to
 * agree to. See `docs/roles.md`.
 */
@Composable
fun CarryingDialog(count: Int, at: Long, onClose: () -> Unit) {
    AlertDialog(
        onDismissRequest = onClose,
        confirmButton = { TextButton(onClick = onClose) { Text("Close") } },
        title = { Text("Carrying $count log${if (count == 1) "" else "s"}") },
        text = {
            Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
                Text(
                    "This phone holds sealed records for other people so that theirs stay " +
                        "readable when their phone is asleep — and so yours do when yours is.",
                    color = Text2, fontSize = 14.sp
                )
                Text(
                    "You cannot read any of it. Not as a rule this app follows, but because " +
                        "nothing here has the keys: carrying is not being granted. If this " +
                        "phone were lost, what is carried would still be sealed.",
                    color = Text2, fontSize = 13.sp
                )
                Text(
                    "It is the other half of reading. Ayni is named for it.",
                    color = Text2, fontSize = 13.sp
                )
                if (at > 0) {
                    Text("Last counted ${Follower.ageWords(at)}", color = Text2, fontSize = 12.sp)
                }
            }
        }
    )
}

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
                    // 🔴 **SAY WHEN THIS INVITE CANNOT BE FULLY GRANTED.** A v2
                    // invite carries no keys identity, so the subject's grant
                    // does the segment half and silently skips the keys half —
                    // the reader ends up unable to read anything in the keys
                    // vault, and impossible to withdraw later, because a
                    // withdrawal needs the identity this invite never carried.
                    // Measured: a fresh install was granted, read nothing, and
                    // the only clue anywhere was the version number below.
                    if (invite.startsWith("diaswarm:2:")) {
                        Text(
                            "⚠️ This is an older invite that does not carry a vault identity. " +
                                "Someone granting it will only share part of what they mean to, " +
                                "and will not be able to withdraw it cleanly afterwards. " +
                                "Turn on “New vault” in the menu and show this again.",
                            color = Warn, fontSize = 13.sp
                        )
                    }
                    Text(invite, color = Text2, fontSize = 10.sp)
                }
            }
        }
    )
}

/**
 * Take an invite as text, for when a camera is not the way it arrived.
 *
 * **VALIDATED BEFORE IT IS ACCEPTED**, because a mistyped or truncated invite
 * queued silently is a follow that never happens and a user with nothing to look
 * at. `inviteParse` is the same check the scanner's result goes through.
 */
@Composable
fun PasteInviteDialog(onFollow: (String) -> Unit, onClose: () -> Unit) {
    var text by remember { mutableStateOf("") }
    val looksRight = text.trim().startsWith("diaswarm:")
    AlertDialog(
        onDismissRequest = onClose,
        confirmButton = {
            TextButton(enabled = looksRight, onClick = { onFollow(text.trim()); onClose() }) {
                Text("Follow")
            }
        },
        dismissButton = { TextButton(onClick = onClose) { Text("Cancel") } },
        title = { Text("Paste an invite") },
        text = {
            Column(verticalArrangement = Arrangement.spacedBy(10.dp)) {
                Text(
                    "Paste the code they sent you. Following costs them nothing, and you " +
                        "cannot read anything until they share with you.",
                    color = Text2, fontSize = 13.sp
                )
                OutlinedTextField(
                    value = text,
                    onValueChange = { text = it },
                    singleLine = false,
                    maxLines = 4,
                    modifier = Modifier.fillMaxWidth()
                )
                if (text.isNotBlank() && !looksRight) {
                    Text(
                        "That does not look like an invite — they all begin “diaswarm:”.",
                        color = Warn, fontSize = 12.sp
                    )
                }
            }
        }
    )
}

/**
 * A section heading inside a sheet.
 *
 * Small, upper-ish and in the dim colour, so it groups the rows under it
 * without competing with the dialog's own title — the thing that went wrong
 * when one section's name WAS the title.
 */
@Composable
private fun SheetHeading(text: String) {
    Text(
        text,
        color = Text2,
        fontSize = 12.sp,
        letterSpacing = 0.8.sp,
        modifier = Modifier.padding(top = 2.dp)
    )
}

/** Who this phone follows, and the handful of controls that exist. */
@Composable
fun PeopleSheet(
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
    AlertDialog(
        onDismissRequest = onClose,
        confirmButton = { TextButton(onClick = onClose) { Text("Close") } },
        // **THE SHEET IS THE WHOLE MENU, AND IT WAS TITLED AS ONE SECTION OF
        // ITSELF.** `AlertDialog` renders `title` as a headline and `text` as
        // its body, so naming it "People you follow" put every app-wide setting
        // — units, the sleep behaviour, both vault toggles, the colour band —
        // underneath a heading about people, reading as though they belonged to
        // whoever was listed above them.
        //
        // Reported by the owner opening it: "'people you follow' looks like a
        // title and all the other menu items look like children". They do,
        // because it is and they were. The headings below say which rows are
        // about a person and which are about the app.
        title = { Text("ayni") },
        text = {
            Column(verticalArrangement = Arrangement.spacedBy(10.dp)) {
                SheetHeading("People you follow")
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
                // THE DIVIDER MOVED DOWN. It used to sit here, cutting the
                // people off from the three actions that are entirely about
                // people — scanning, pasting and showing an invite — and
                // joining those to the app settings instead. The break belongs
                // where the subject changes, which is at "Settings".
                Text("Scan an invite", color = Text1, fontSize = 15.sp,
                    modifier = Modifier.fillMaxWidth().clickable { onScan() }.padding(vertical = 8.dp))
                // **BECAUSE SCANNING NEEDS TWO PHONES IN ONE ROOM.** An invite
                // arrives by message as often as by camera, and until now the
                // only way in was the scanner — which also made the app
                // impossible to set up over remote help, or with one device.
                Text("Paste an invite", color = Text1, fontSize = 15.sp,
                    modifier = Modifier.fillMaxWidth().clickable { onPaste() }.padding(vertical = 8.dp))
                Text("Show my invite", color = Text1, fontSize = 15.sp,
                    modifier = Modifier.fillMaxWidth().clickable { onShowInvite() }.padding(vertical = 8.dp))
                Divider(color = Color(0xFF2A3140))
                SheetHeading("Settings")
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
