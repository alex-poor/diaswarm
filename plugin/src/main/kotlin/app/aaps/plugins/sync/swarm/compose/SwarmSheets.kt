package app.aaps.plugins.sync.swarm.compose

import android.graphics.Bitmap
import android.os.Bundle
import android.view.Gravity
import android.view.LayoutInflater
import android.view.View
import android.view.ViewGroup
import androidx.compose.foundation.Image
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.platform.ComposeView
import androidx.compose.ui.platform.ViewCompositionStrategy
import androidx.compose.ui.unit.dp
import androidx.fragment.app.DialogFragment
import androidx.fragment.app.FragmentActivity
import app.aaps.core.compose.components.ListRow
import app.aaps.core.compose.components.PrimaryButton
import app.aaps.core.compose.components.SheetSurface
import app.aaps.core.compose.theme.AapsTheme

/**
 * One host for every swarm bottom sheet: a `DialogFragment` whose window is
 * pinned to the bottom with a transparent background, exactly as the app's own
 * action sheets are (ProfileSwitchDialog et al.). The content is a composable
 * handed in via [show]; it receives a `dismiss` it can wire to a close button.
 *
 * Transient by design — on process death [sheet] is null and the fragment just
 * dismisses, which is the right behaviour for a momentary sheet.
 */
class SwarmSheetDialog : DialogFragment() {
    var sheet: (@Composable (dismiss: () -> Unit) -> Unit)? = null

    override fun onStart() {
        super.onStart()
        dialog?.window?.apply {
            setLayout(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT)
            setGravity(Gravity.BOTTOM)
            setBackgroundDrawableResource(android.R.color.transparent)
        }
    }

    override fun onCreateView(inflater: LayoutInflater, container: ViewGroup?, savedInstanceState: Bundle?): View {
        val s = sheet ?: run { dismiss(); return View(requireContext()) }
        return ComposeView(requireContext()).apply {
            setViewCompositionStrategy(ViewCompositionStrategy.DisposeOnViewTreeLifecycleDestroyed)
            setContent { AapsTheme { s { dismiss() } } }
        }
    }

    companion object {
        fun show(activity: FragmentActivity, tag: String, sheet: @Composable (dismiss: () -> Unit) -> Unit) {
            SwarmSheetDialog().apply { this.sheet = sheet }.show(activity.supportFragmentManager, tag)
        }
    }
}

/** The invite to hand over: a QR to scan, the plain explanation, and Copy. */
@Composable
fun InviteSheet(
    title: String,
    qr: Bitmap?,
    whyNot: String?,
    explain: String,
    copyLabel: String,
    onCopy: () -> Unit,
    onClose: () -> Unit
) {
    SheetSurface(title = title, onClose = onClose) {
        if (qr == null) {
            Text(whyNot ?: "", style = AapsTheme.type.body, color = AapsTheme.colors.textSecondary)
        } else {
            Column(
                horizontalAlignment = Alignment.CenterHorizontally,
                verticalArrangement = Arrangement.spacedBy(12.dp)
            ) {
                // FULL WIDTH, NOT A FIXED 240.dp. The code carries a key bundle
                // and is dense; shown small, another phone's camera could not
                // resolve the modules. Drawn edge to edge (square) it reads like
                // the follower's own invite does.
                Image(
                    bitmap = qr.asImageBitmap(),
                    contentDescription = null,
                    modifier = Modifier.fillMaxWidth().aspectRatio(1f)
                )
                Text(explain, style = AapsTheme.type.body, color = AapsTheme.colors.textSecondary)
                PrimaryButton(copyLabel, onCopy, Modifier.fillMaxWidth())
            }
        }
    }
}

/** One followed subject: a label, and a freshness/status line. */
data class SwarmFollowingRow(val label: String, val status: String)

/** Whose data this phone carries, and how fresh each is. Read-only. */
@Composable
fun FollowingSheet(
    title: String,
    rows: List<SwarmFollowingRow>,
    empty: String,
    onClose: () -> Unit
) {
    SheetSurface(title = title, onClose = onClose) {
        if (rows.isEmpty()) {
            Text(empty, style = AapsTheme.type.body, color = AapsTheme.colors.textSecondary)
        } else {
            Column(verticalArrangement = Arrangement.spacedBy(2.dp)) {
                rows.forEach { r -> ListRow(title = r.label, sub = r.status) }
            }
        }
    }
}
