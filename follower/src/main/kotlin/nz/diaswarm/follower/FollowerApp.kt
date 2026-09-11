package nz.diaswarm.follower

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.RoundedCornerShape
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
private val Insulin = Color(0xFF63A8F0)
private val Carbs = Color(0xFFE0A33F)

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
    val treatments = remember(tick, subject) {
        subject?.let { Follower.treatments(context, it, Prefs.rangeHours(context)) } ?: emptyList()
    }
    val target = remember(tick, subject) { subject?.let { Follower.target(context, it) } }
    val followed = remember(tick) { Follower.following(context) }
    val low = remember(tick) { Prefs.lowLine(context) }
    val high = remember(tick) { Prefs.highLine(context) }
    val scheduled = remember(tick, subject) {
        subject?.let { Follower.scheduledBasal(context, it, System.currentTimeMillis()) } ?: 0.0
    }
    val basal = remember(tick, subject, treatments) {
        if (readings.isEmpty()) emptyList()
        else Follower.basalSteps(treatments, scheduled, readings.first().at, readings.last().at)
    }

    MaterialTheme(colorScheme = darkColorScheme(background = Bg, surface = Card)) {
        // NOT A SCROLLER ANY MORE, AND THAT IS WHAT FILLS THE SCREEN. A
        // `verticalScroll` column gives every child its intrinsic height, so a
        // chart pinned at 260.dp left roughly a third of a phone blank below it
        // and scrolled to reveal nothing. Without the scroll, `weight(1f)` can
        // hand the chart everything the cards above it did not take.
        Column(
            Modifier.fillMaxSize().background(Bg)
                .padding(horizontal = 16.dp).padding(top = 28.dp),
            verticalArrangement = Arrangement.spacedBy(12.dp)
        ) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Text("ayni", color = Text2, fontSize = 13.sp, modifier = Modifier.weight(1f))
                Text("···", color = Text2, fontSize = 20.sp,
                    modifier = Modifier.clip(RoundedCornerShape(8.dp)).clickable { showPeople = true }.padding(8.dp))
            }

            when {
                followed.isEmpty() -> EmptyCard(onScan = onScan, onShowInvite = { Endpoint.expectOffer(); showInvite = true })
                subject == null    -> ChooseCard(followed) { Prefs.setGraphSubject(context, it.key); tick++ }
                readings.isEmpty() -> WaitingCard(subject)
                else               -> {
                    HeroCard(context, readings, treatments, target, low, high)
                    Card(
                        colors = CardDefaults.cardColors(containerColor = Card),
                        modifier = Modifier.fillMaxWidth().weight(1f)
                    ) {
                        Column(Modifier.padding(12.dp).fillMaxSize()) {
                            RangeRow(
                                hours = Prefs.rangeHours(context),
                                // SAYS WHOSE RANGE IT IS. The colours are judged
                                // against this phone's setting, not against the
                                // subject's target, and a chart that let anyone
                                // assume otherwise would be attributing a
                                // threshold to them that they never published.
                                caption = "in range ${Prefs.show(context, low)}–${Prefs.show(context, high)}" +
                                    " · this phone",
                                onPick = { Prefs.setRangeHours(context, it); tick++ }
                            )
                            GlucoseChart(
                                readings, treatments, basal, scheduled, target,
                                low, high, Prefs.mmol(context), Modifier.fillMaxSize()
                            )
                        }
                    }
                }
            }
            Spacer(Modifier.height(16.dp))
        }
    }

    if (showInvite) InviteDialog(Endpoint.invite(context)) { showInvite = false }
    if (showPeople) PeopleSheet(
        followed = followed,
        onScan = { showPeople = false; onScan() },
        onShowInvite = { showPeople = false; Endpoint.expectOffer(); showInvite = true },
        onChoose = { Prefs.setGraphSubject(context, it.key); tick++; showPeople = false },
        onUnits = { Prefs.setMmol(context, !Prefs.mmol(context)); tick++ },
        onBand = { Prefs.cycleBand(context); tick++ },
        bandLabel = Prefs.bandLabel(context),
        mmol = Prefs.mmol(context),
        onClose = { showPeople = false }
    )
}

@Composable
private fun HeroCard(
    context: android.content.Context,
    readings: List<Follower.Reading>,
    treatments: List<Follower.Treatment>,
    target: Pair<Double, Double>?,
    lowLine: Double,
    highLine: Double
) {
    val last = readings.last()
    val prev = readings.getOrNull(readings.size - 2)
    val delta = prev?.let { last.mgdl - it.mgdl }

    Card(colors = CardDefaults.cardColors(containerColor = Card), modifier = Modifier.fillMaxWidth()) {
        Column(Modifier.padding(18.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
            // AGE FIRST AND ALWAYS. Never a bare number.
            Text(Follower.ageWords(last.at), color = Text2, fontSize = 13.sp,
                modifier = Modifier.fillMaxWidth(), textAlign = androidx.compose.ui.text.style.TextAlign.End)
            // THE SAME RULE AS THE CHART, from the same function. These
            // disagreed once — the number here was unconditionally green while
            // the identical reading was drawn red below it, because the chart
            // classified against a single-point target and this did not
            // classify at all.
            val tone = readingColor(last.mgdl, lowLine, highLine)
            Row(verticalAlignment = Alignment.Bottom, horizontalArrangement = Arrangement.spacedBy(10.dp)) {
                Text(Prefs.show(context, last.mgdl), color = tone, fontSize = 56.sp,
                    fontWeight = FontWeight.Bold, lineHeight = 58.sp)
                delta?.let {
                    val sign = if (it >= 0) "+" else "−"
                    Text("$sign${Prefs.show(context, kotlin.math.abs(it))}", color = tone,
                        fontSize = 18.sp, modifier = Modifier.padding(bottom = 12.dp))
                }
            }
            target?.let { (lo, hi) ->
                Text(
                    "target ${Prefs.show(context, lo)}–${Prefs.show(context, hi)} ${Prefs.unitLabel(context)}",
                    color = Text2, fontSize = 13.sp
                )
            }

            // WHAT WAS DONE, AND WHEN — the two questions a person checking on
            // somebody actually has after the number itself. Both are reports
            // of records that were sent; a kind with nothing in the window is
            // simply absent, because "0 U" would be a claim that nothing was
            // delivered rather than that nothing was heard about.
            val lastBolus = treatments.filter { it.kind == "bolus" || it.kind == "extbolus" }.maxByOrNull { it.at }
            val lastCarb = treatments.filter { it.kind == "carb" }.maxByOrNull { it.at }
            if (lastBolus != null || lastCarb != null) {
                Row(
                    horizontalArrangement = Arrangement.spacedBy(18.dp),
                    modifier = Modifier.padding(top = 4.dp)
                ) {
                    lastBolus?.let {
                        Text("${trimUnits(it.value)} U · ${Follower.ageWords(it.at)}",
                            color = Insulin, fontSize = 13.sp)
                    }
                    lastCarb?.let {
                        Text("${trimUnits(it.value)} g · ${Follower.ageWords(it.at)}",
                            color = Carbs, fontSize = 13.sp)
                    }
                }
            }
        }
    }
}

/** Matches the chart's own labels: a dose is not improved by trailing zeros. */
private fun trimUnits(v: Double): String =
    if (kotlin.math.abs(v - v.toInt()) < 0.005) "${v.toInt()}"
    else String.format("%.2f", v).trimEnd('0').trimEnd('.')

@Composable
private fun RangeRow(hours: Int, caption: String, onPick: (Int) -> Unit) {
    Row(
        horizontalArrangement = Arrangement.spacedBy(8.dp),
        verticalAlignment = Alignment.CenterVertically,
        modifier = Modifier.fillMaxWidth().padding(bottom = 2.dp)
    ) {
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
        Spacer(Modifier.weight(1f))
    }
    // ON ITS OWN LINE, because it does not fit beside the chips. Squeezed into
    // the end of that row it wrapped to two lines on a Pixel 7, and it will be
    // longer in mg/dL ("in range 70-180") and longer again translated.
    Text(caption, color = Text2, fontSize = 11.sp, maxLines = 1,
        modifier = Modifier.padding(start = 12.dp, bottom = 6.dp))
}

@Composable
private fun EmptyCard(onScan: () -> Unit, onShowInvite: () -> Unit) {
    Card(colors = CardDefaults.cardColors(containerColor = Card), modifier = Modifier.fillMaxWidth()) {
        Column(Modifier.padding(18.dp), verticalArrangement = Arrangement.spacedBy(14.dp)) {
            Text("Not following anyone yet.", color = Text1, fontSize = 17.sp)
            Text(
                "Ask them to show you their invite, then scan it. They decide what " +
                    "you can read, and can stop at any time.",
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
