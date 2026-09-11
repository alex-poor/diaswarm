package nz.diaswarm.follower

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import kotlinx.coroutines.delay

private val Bg = Color(0xFF0E1116)
private val Card = Color(0xFF171B22)
private val Text1 = Color(0xFFE8EAED)
private val Text2 = Color(0xFF9AA3B0)
private val Good = Color(0xFF34C38F)

@Composable
fun FollowerApp(onScan: () -> Unit, scanned: String?, onScanHandled: () -> Unit) {
    val context = LocalContext.current
    var tick by remember { mutableStateOf(0) }
    var showInvite by remember { mutableStateOf(false) }
    var showPeople by remember { mutableStateOf(false) }

    // Redraw on a clock, because the AGE changes even when the data does not —
    // and a stale reading that still says "1 min ago" is the failure §12.3 is
    // about.
    LaunchedEffect(Unit) { while (true) { delay(15_000); tick++ } }

    // AND ACTUALLY FETCH WHILE SOMEBODY IS WATCHING. The periodic job's floor is
    // fifteen minutes, which is useless to a person looking at the screen: a
    // reading arrives every minute or five and they would see it a quarter of an
    // hour later. This was exactly the symptom — an app open in the hand,
    // showing a number six minutes old and never moving.
    //
    // Driven from the UI rather than chained from inside the worker on purpose.
    // A worker that re-enqueues its OWN unique name with REPLACE cancels itself,
    // which is a silent stall that looks precisely like a quiet network.
    LaunchedEffect(Unit) {
        while (true) {
            Sync.now(context)
            delay(SyncWorker.EVERY_SECONDS * 1000)
        }
    }

    scanned?.let { text ->
        LaunchedEffect(text) {
            Invites.queue(context, text)
            onScanHandled()
            tick++
        }
    }

    val subject = remember(tick) { Follower.chosen(context) }
    val readings = remember(tick, subject) {
        subject?.let { Follower.readings(context, it, Prefs.rangeHours(context)) } ?: emptyList()
    }
    val target = remember(tick, subject) { subject?.let { Follower.target(context, it) } }
    val followed = remember(tick) { Follower.following(context) }

    MaterialTheme(colorScheme = darkColorScheme(background = Bg, surface = Card)) {
        Column(
            Modifier.fillMaxSize().background(Bg).verticalScroll(rememberScrollState())
                .padding(horizontal = 16.dp).padding(top = 28.dp),
            verticalArrangement = Arrangement.spacedBy(12.dp)
        ) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Text("diaswarm", color = Text2, fontSize = 13.sp, modifier = Modifier.weight(1f))
                Text("···", color = Text2, fontSize = 20.sp,
                    modifier = Modifier.clip(RoundedCornerShape(8.dp)).clickable { showPeople = true }.padding(8.dp))
            }

            when {
                followed.isEmpty() -> EmptyCard(onScan = onScan, onShowInvite = { Endpoint.expectOffer(); showInvite = true })
                subject == null    -> ChooseCard(followed) { Prefs.setGraphSubject(context, it.key); tick++ }
                readings.isEmpty() -> WaitingCard(subject)
                else               -> {
                    HeroCard(context, readings, target)
                    Card(colors = CardDefaults.cardColors(containerColor = Card), modifier = Modifier.fillMaxWidth()) {
                        Column(Modifier.padding(12.dp)) {
                            RangeRow(Prefs.rangeHours(context)) { Prefs.setRangeHours(context, it); tick++ }
                            GlucoseChart(readings, target, Prefs.mmol(context))
                        }
                    }
                }
            }
            Spacer(Modifier.height(24.dp))
        }
    }

    if (showInvite) InviteDialog(Endpoint.invite(context)) { showInvite = false }
    if (showPeople) PeopleSheet(
        followed = followed,
        onScan = { showPeople = false; onScan() },
        onShowInvite = { showPeople = false; Endpoint.expectOffer(); showInvite = true },
        onChoose = { Prefs.setGraphSubject(context, it.key); tick++; showPeople = false },
        onUnits = { Prefs.setMmol(context, !Prefs.mmol(context)); tick++ },
        mmol = Prefs.mmol(context),
        onClose = { showPeople = false }
    )
}

@Composable
private fun HeroCard(
    context: android.content.Context,
    readings: List<Follower.Reading>,
    target: Pair<Double, Double>?
) {
    val last = readings.last()
    val prev = readings.getOrNull(readings.size - 2)
    val delta = prev?.let { last.mgdl - it.mgdl }

    Card(colors = CardDefaults.cardColors(containerColor = Card), modifier = Modifier.fillMaxWidth()) {
        Column(Modifier.padding(18.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
            // AGE FIRST AND ALWAYS. Never a bare number.
            Text(Follower.ageWords(last.at), color = Text2, fontSize = 13.sp,
                modifier = Modifier.fillMaxWidth(), textAlign = androidx.compose.ui.text.style.TextAlign.End)
            Row(verticalAlignment = Alignment.Bottom, horizontalArrangement = Arrangement.spacedBy(10.dp)) {
                Text(Prefs.show(context, last.mgdl), color = Good, fontSize = 56.sp,
                    fontWeight = FontWeight.Bold, lineHeight = 58.sp)
                delta?.let {
                    val sign = if (it >= 0) "+" else "−"
                    Text("$sign${Prefs.show(context, kotlin.math.abs(it))}", color = Good,
                        fontSize = 18.sp, modifier = Modifier.padding(bottom = 12.dp))
                }
            }
            target?.let { (lo, hi) ->
                Text(
                    "target ${Prefs.show(context, lo)}–${Prefs.show(context, hi)} ${Prefs.unitLabel(context)}",
                    color = Text2, fontSize = 13.sp
                )
            }
        }
    }
}

@Composable
private fun RangeRow(hours: Int, onPick: (Int) -> Unit) {
    Row(horizontalArrangement = Arrangement.spacedBy(8.dp), modifier = Modifier.padding(bottom = 8.dp)) {
        listOf(3, 6, 12, 24).forEach { h ->
            val on = h == hours
            Text(
                "${h}h",
                color = if (on) Text1 else Text2,
                fontSize = 13.sp,
                modifier = Modifier.clip(RoundedCornerShape(20.dp))
                    .background(if (on) Color(0xFF2A3140) else Color.Transparent)
                    .clickable { onPick(h) }
                    .padding(horizontal = 12.dp, vertical = 6.dp)
            )
        }
    }
}

@Composable
private fun EmptyCard(onScan: () -> Unit, onShowInvite: () -> Unit) {
    Card(colors = CardDefaults.cardColors(containerColor = Card), modifier = Modifier.fillMaxWidth()) {
        Column(Modifier.padding(18.dp), verticalArrangement = Arrangement.spacedBy(14.dp)) {
            Text("Not following anyone yet.", color = Text1, fontSize = 17.sp)
            Text(
                "Ask them to open diaswarm on their phone and show you their invite, " +
                    "then scan it. They decide what you can read, and can stop at any time.",
                color = Text2, fontSize = 14.sp
            )
            Button(onClick = onScan, modifier = Modifier.fillMaxWidth()) { Text("Scan their invite") }
            OutlinedButton(onClick = onShowInvite, modifier = Modifier.fillMaxWidth()) { Text("Show my invite") }
        }
    }
}

@Composable
private fun ChooseCard(followed: List<Follower.Subject>, onChoose: (Follower.Subject) -> Unit) {
    Card(colors = CardDefaults.cardColors(containerColor = Card), modifier = Modifier.fillMaxWidth()) {
        Column(Modifier.padding(18.dp), verticalArrangement = Arrangement.spacedBy(10.dp)) {
            Text("Whose glucose should this show?", color = Text1, fontSize = 17.sp)
            // Picking one at random would be picking a PERSON at random.
            followed.forEach { s ->
                Text(s.short, color = Good, fontSize = 15.sp,
                    modifier = Modifier.fillMaxWidth().clip(RoundedCornerShape(8.dp))
                        .clickable { onChoose(s) }.padding(vertical = 10.dp))
            }
        }
    }
}

@Composable
private fun WaitingCard(subject: Follower.Subject) {
    Card(colors = CardDefaults.cardColors(containerColor = Card), modifier = Modifier.fillMaxWidth()) {
        Column(Modifier.padding(18.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
            Text("Following ${subject.short}", color = Text1, fontSize = 17.sp)
            Text(
                "Nothing readable yet. Following costs them nothing and grants you nothing — " +
                    "they still have to share with you from their own phone.",
                color = Text2, fontSize = 14.sp
            )
        }
    }
}
