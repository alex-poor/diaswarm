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

// Shared tokens (see Theme.kt); Warn is the amber for the two places a screen
// has to say "this will not do what you think".
private val Text1 = AyniColor.Text1
private val Text2 = AyniColor.Text2
private val Warn = AyniColor.Warn

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
            }) { Text("Copy code") }
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
                        "It is not a secret: just a public key and an address. Show it to the " +
                            "person whose readings you want to follow, and they can add you as a " +
                            "reader. On its own it unlocks nothing — only they can choose to share.",
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
                    // The raw invite is NOT printed here any more. A wall of hex
                    // read as a secret to guard and made the dialog look like a
                    // diagnostic, not a thing to hand over — the point is the
                    // code above and the one tap that copies it ("Copy code").
                    // It still goes on the clipboard in full for the channels a
                    // camera cannot reach.
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
 * Rename a followed person — a name for your eyes only.
 *
 * **IT IS A LOCAL LABEL, NOT PART OF THE PROTOCOL.** Nothing here is sent to the
 * subject or any carrier; it only changes what this phone writes to
 * [Prefs.setHandleFor] for this subject's key. [current] is whatever is stored
 * now (the invite's handle, a previous edit, or empty), prefilled so a rename is
 * an edit rather than a retype. Clearing it and saving falls the list back to
 * the short hex — the sole place that key is ever shown. 40-char cap matches the
 * store; the subject's own key is shown so there is no doubt who is being named.
 */
@Composable
fun EditNameDialog(current: String, subjectShort: String, onSave: (String) -> Unit, onClose: () -> Unit) {
    var text by remember { mutableStateOf(current) }
    AlertDialog(
        onDismissRequest = onClose,
        confirmButton = { TextButton(onClick = { onSave(text.trim()); onClose() }) { Text("Save") } },
        dismissButton = { TextButton(onClick = onClose) { Text("Cancel") } },
        title = { Text("Name this person") },
        text = {
            Column(verticalArrangement = Arrangement.spacedBy(10.dp)) {
                Text(
                    "A name for your eyes only — it is never sent to them or anyone " +
                        "carrying their data. Clear it to go back to showing $subjectShort.",
                    color = Text2, fontSize = 13.sp
                )
                OutlinedTextField(
                    value = text,
                    onValueChange = { text = it.take(40) },
                    singleLine = true,
                    modifier = Modifier.fillMaxWidth()
                )
            }
        }
    )
}
