package app.aaps.plugins.sync.swarm.compose

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.runtime.toMutableStateList
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.dp
import app.aaps.core.compose.components.ListRow
import app.aaps.core.compose.components.SheetSurface
import app.aaps.core.compose.theme.AapsTheme
import app.aaps.core.ui.dialogs.OKDialog
import app.aaps.core.ui.extensions.scanForActivity

/** One reader as the sheet shows it: a label to recognise, a line of detail, and the key to revoke by. */
data class SwarmReaderRow(val key: String, val label: String, val detail: String)

/** The strings the sheet needs, resolved by the caller (which has the resources). */
data class ReadersSheetStrings(
    val title: String,
    val empty: String,
    val footer: String,
    val stop: String,
    val confirmTitle: String,
    val explain: String
)

/**
 * "Who can read" as a bottom sheet in the app's own design system, with a Stop
 * action on each reader's row — no copying a key into another screen.
 *
 * The row list is held in [remember] so a withdrawal removes it at once; the
 * sync pass that the caller's `onStop` kicks off then confirms on the keys
 * group and in the book, and toasts.
 */
@Composable
fun ReadersSheet(
    initial: List<SwarmReaderRow>,
    strings: ReadersSheetStrings,
    onStop: (SwarmReaderRow) -> Unit,
    onClose: () -> Unit
) {
    val rows = remember { initial.toMutableStateList() }
    val activity = LocalContext.current.scanForActivity()
    SheetSurface(title = strings.title, onClose = onClose) {
        if (rows.isEmpty()) {
            Text(strings.empty, style = AapsTheme.type.body, color = AapsTheme.colors.textSecondary)
        } else {
            Column(verticalArrangement = Arrangement.spacedBy(2.dp)) {
                rows.forEach { r ->
                    ListRow(
                        title = r.label,
                        sub = r.detail,
                        trailing = {
                            StopPill(strings.stop) {
                                val confirm = Runnable { onStop(r); rows.remove(r) }
                                if (activity != null)
                                    OKDialog.showConfirmation(
                                        activity, strings.confirmTitle.format(r.label), strings.explain, confirm
                                    )
                                else confirm.run()
                            }
                        }
                    )
                }
                Text(
                    strings.footer,
                    style = AapsTheme.type.caption,
                    color = AapsTheme.colors.textSecondary,
                    modifier = Modifier.padding(top = 8.dp)
                )
            }
        }
    }
}

/** A compact, danger-tinted pill — the row action, since [app.aaps.core.compose.components.DangerButton] is full-width. */
@Composable
private fun StopPill(label: String, onClick: () -> Unit) {
    Text(
        label,
        style = AapsTheme.type.caption,
        color = AapsTheme.colors.low,
        modifier = Modifier
            .clip(AapsTheme.shape.pill)
            .background(AapsTheme.colors.controlFill)
            .clickable(onClick = onClick)
            .padding(horizontal = 14.dp, vertical = 8.dp)
    )
}
