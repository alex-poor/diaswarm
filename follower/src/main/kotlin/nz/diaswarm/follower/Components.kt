package nz.diaswarm.follower

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.Edit
import androidx.compose.material3.*
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp

/**
 * The reusable pieces of Ayni's look. A screen assembles these rather than
 * hand-rolling a Row with its own padding and colours each time — which is how
 * the settings sheet ended up a wall of tappable Text with no two rows quite
 * alike. Everything here draws from the tokens in Theme.kt.
 *
 * Each row takes an optional leading [ImageVector], tinted dim so it labels the
 * row without competing with the text. These are real Material vectors, not the
 * Unicode glyphs an earlier version of this comment refused — a glyph that
 * renders differently on every phone is worse than none, a drawn vector is not.
 */

/** A rounded surface card with the standard inset. */
@Composable
fun AyniCard(modifier: Modifier = Modifier, content: @Composable ColumnScope.() -> Unit) {
    Card(
        colors = CardDefaults.cardColors(containerColor = AyniColor.Card),
        shape = RoundedCornerShape(AyniRadius.Card),
        modifier = modifier.fillMaxWidth()
    ) { Column(Modifier.padding(AyniSpace.s4), content = content) }
}

/** Upper-cased section label in the dim colour. */
@Composable
fun SectionHeading(text: String) {
    Text(
        text.uppercase(),
        color = AyniColor.Text3, fontSize = AyniType.Overline, letterSpacing = 1.sp,
        modifier = Modifier.padding(start = AyniSpace.s1, top = AyniSpace.s5, bottom = AyniSpace.s2)
    )
}

/** A heading followed by a card that holds the rows placed in [content]. */
@Composable
fun SettingsGroup(title: String, content: @Composable ColumnScope.() -> Unit) {
    SectionHeading(title)
    Card(
        colors = CardDefaults.cardColors(containerColor = AyniColor.Card),
        shape = RoundedCornerShape(AyniRadius.Card),
        modifier = Modifier.fillMaxWidth()
    ) { Column(content = content) }
}

/** A hairline between rows in a group, inset to line up under the titles. */
@Composable
fun RowDivider() = HorizontalDivider(
    color = AyniColor.Line, thickness = 1.dp,
    modifier = Modifier.padding(horizontal = AyniSpace.s4)
)

@Composable
private fun RowShell(onClick: (() -> Unit)?, leading: ImageVector? = null, content: @Composable RowScope.() -> Unit) {
    val base = Modifier.fillMaxWidth()
    Row(
        (if (onClick != null) base.clickable { onClick() } else base)
            .padding(horizontal = AyniSpace.s4, vertical = 13.dp),
        verticalAlignment = Alignment.CenterVertically
    ) {
        if (leading != null) {
            Icon(leading, contentDescription = null, tint = AyniColor.Text3,
                modifier = Modifier.size(20.dp))
            Spacer(Modifier.width(AyniSpace.s3))
        }
        content()
    }
}

@Composable
private fun RowScope.TitleAndSubtitle(title: String, subtitle: String?, titleColor: androidx.compose.ui.graphics.Color = AyniColor.Text1) {
    Column(Modifier.weight(1f)) {
        Text(title, color = titleColor, fontSize = AyniType.Body)
        if (subtitle != null) Text(subtitle, color = AyniColor.Text2, fontSize = 12.sp,
            modifier = Modifier.padding(top = 2.dp))
    }
}

/** A row with a real Material switch on the right. The whole row toggles. */
@Composable
fun SwitchRow(title: String, subtitle: String?, checked: Boolean, leading: ImageVector? = null, onToggle: () -> Unit) {
    RowShell(onClick = onToggle, leading = leading) {
        TitleAndSubtitle(title, subtitle)
        Spacer(Modifier.width(AyniSpace.s3))
        Switch(
            checked = checked, onCheckedChange = { onToggle() },
            colors = SwitchDefaults.colors(
                checkedThumbColor = androidx.compose.ui.graphics.Color.White,
                checkedTrackColor = AyniColor.InRange,
                checkedBorderColor = AyniColor.InRange,
                uncheckedThumbColor = AyniColor.Text2,
                uncheckedTrackColor = AyniColor.CardRaised,
                uncheckedBorderColor = AyniColor.Line
            )
        )
    }
}

/** A row showing a current value on the right (e.g. "mmol/L"), tappable to change. */
@Composable
fun ValueRow(title: String, subtitle: String?, value: String, leading: ImageVector? = null, onClick: () -> Unit) {
    RowShell(onClick = onClick, leading = leading) {
        TitleAndSubtitle(title, subtitle)
        Spacer(Modifier.width(AyniSpace.s3))
        Text(value, color = AyniColor.Text2, fontSize = AyniType.BodySmall)
    }
}

/** A row that performs an action, with a chevron to say so. */
@Composable
fun ActionRow(title: String, subtitle: String? = null, leading: ImageVector? = null, onClick: () -> Unit) {
    RowShell(onClick = onClick, leading = leading) {
        TitleAndSubtitle(title, subtitle)
        Spacer(Modifier.width(AyniSpace.s3))
        Text("›", color = AyniColor.Text3, fontSize = 22.sp)
    }
}

/**
 * A person in the follow list: name + what this phone has of theirs.
 *
 * [onEdit], when given, draws a trailing pencil that renames them. It is its own
 * tappable target inside the row, so the pencil renames while the rest of the
 * row still does [onClick] — a child clickable consumes its own tap.
 */
@Composable
fun SubjectRow(title: String, status: String, selected: Boolean, leading: ImageVector? = null, onEdit: (() -> Unit)? = null, onClick: () -> Unit) {
    RowShell(onClick = onClick, leading = leading) {
        Column(Modifier.weight(1f)) {
            Text(title, color = AyniColor.Text1, fontSize = AyniType.Body,
                fontWeight = if (selected) FontWeight.SemiBold else FontWeight.Normal)
            Text(status, color = AyniColor.Text2, fontSize = 12.sp, modifier = Modifier.padding(top = 2.dp))
        }
        if (selected) {
            Spacer(Modifier.width(AyniSpace.s3))
            Box(Modifier.size(8.dp).clip(RoundedCornerShape(4.dp)).background(AyniColor.InRange))
        }
        if (onEdit != null) {
            Spacer(Modifier.width(AyniSpace.s2))
            IconButton(onClick = onEdit, modifier = Modifier.size(32.dp)) {
                Icon(Icons.Outlined.Edit, contentDescription = "Rename", tint = AyniColor.Text3,
                    modifier = Modifier.size(19.dp))
            }
        }
    }
}

/** The tone of a status banner. */
enum class BannerTone { Alert, Wait }

/** A one-line status strip, used for alerts and the waiting/pending states. */
@Composable
fun StatusBanner(tone: BannerTone, text: String, modifier: Modifier = Modifier) {
    val bg: androidx.compose.ui.graphics.Color
    val fg: androidx.compose.ui.graphics.Color
    val dot: androidx.compose.ui.graphics.Color
    when (tone) {
        BannerTone.Alert -> { bg = AyniColor.Urgent.copy(alpha = 0.14f); fg = AyniColor.Text1; dot = AyniColor.Urgent }
        BannerTone.Wait -> { bg = AyniColor.CardRaised; fg = AyniColor.Text2; dot = AyniColor.Text2 }
    }
    Row(
        modifier.fillMaxWidth()
            .clip(RoundedCornerShape(AyniRadius.Row))
            .background(bg)
            .padding(horizontal = 13.dp, vertical = 11.dp),
        verticalAlignment = Alignment.CenterVertically
    ) {
        Box(Modifier.size(9.dp).clip(RoundedCornerShape(5.dp)).background(dot))
        Spacer(Modifier.width(AyniSpace.s2 + 2.dp))
        Text(text, color = fg, fontSize = AyniType.Caption)
    }
}
