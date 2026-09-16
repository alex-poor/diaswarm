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
    var showPaste by remember { mutableStateOf(false) }
    var showCarrying by remember { mutableStateOf(false) }

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
    // Read separately from [target] so the card can SAY it is temporary. A band
    // that silently changes looks like they edited their profile.
    val tempTarget = remember(tick, subject) { subject?.let { Follower.runningTempTarget(context, it) } }
    val followed = remember(tick) { Follower.following(context) }
    val carrying = remember(tick) { Prefs.carrying(context) }
    val low = remember(tick) { Prefs.lowLine(context) }
    val high = remember(tick) { Prefs.highLine(context) }
    // NULLABLE ON PURPOSE — see [Follower.scheduledBasalOrNull]. Null is "this
    // phone has no profile for them", which is not the same statement as a
    // basal rate of zero, and the chart must not turn one into the other.
    val scheduled = remember(tick, subject) {
        subject?.let { Follower.scheduledBasalOrNull(context, it, System.currentTimeMillis()) }
    }
    val basal = remember(tick, subject, treatments) {
        if (readings.isEmpty()) emptyList()
        else Follower.basalSteps(treatments, scheduled, readings.first().at, readings.last().at)
    }
    val basalUnknown = remember(tick, subject, treatments) {
        scheduled == null && treatments.any { it.kind == "tbr" && !it.absolute }
    }

    MaterialTheme(colorScheme = darkColorScheme(background = Bg, surface = Card)) {
        // NOT A SCROLLER ANY MORE, AND THAT IS WHAT FILLS THE SCREEN. A
        // `verticalScroll` column gives every child its intrinsic height, so a
        // chart pinned at 260.dp left roughly a third of a phone blank below it
        // and scrolled to reveal nothing. Without the scroll, `weight(1f)` can
        // hand the chart everything the cards above it did not take.
        Column(
            // **THE STATUS BAR IS 136px AND THIS USED TO RESERVE 28.dp.** At
            // this phone's density that is 74px, so the whole top row — the
            // name, the carrying count, and the `···` that opens every setting
            // in the app — was drawn underneath the status bar. The glyph is
            // visible there, because the app draws edge to edge and the status
            // bar is transparent. It is not reliably tappable: the status bar
            // window takes the touch, and only the part of the button below
            // 136px reaches Compose at all.
            //
            // Found by driving it with `adb input tap`: a tap on the dots does
            // nothing and a tap ten pixels lower opens the sheet. A person
            // holding the phone gets the same behaviour and no explanation —
            // "I tap the ... and nothing happens" — which is the report that
            // started `7c62da4`, arriving from a second direction. That one was
            // the sheet rendering invisibly; this is the button that opens it.
            //
            // `statusBars` rather than a bigger constant, because the number is
            // the device's and not ours: a phone with a taller cutout moves it
            // again and a constant would be wrong somewhere else instead.
            Modifier.fillMaxSize().background(Bg)
                .windowInsetsPadding(WindowInsets.statusBars)
                .padding(horizontal = 16.dp).padding(top = 12.dp),
            verticalArrangement = Arrangement.spacedBy(12.dp)
        ) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Text("ayni", color = Text2, fontSize = 13.sp)
                // **NEXT TO THE NAME, BECAUSE THE NAME IS WHAT IT MEANS.** D30
                // makes carrying the price of reading, and until now it
                // happened only in the log — so a person could run this for
                // months without knowing they held anyone else's ciphertext.
                // An invariant nobody can see is one nobody agreed to.
                //
                // Deliberately small and always present rather than prominent
                // and dismissible: it is a standing fact about what this phone
                // is doing, not a notification.
                if (carrying > 0) {
                    Text(
                        " · carrying $carrying",
                        color = Text2, fontSize = 13.sp,
                        modifier = Modifier.clip(RoundedCornerShape(8.dp))
                            .clickable { showCarrying = true }.padding(horizontal = 4.dp)
                    )
                }
                Spacer(Modifier.weight(1f))
                Text("···", color = Text2, fontSize = 20.sp,
                    modifier = Modifier.clip(RoundedCornerShape(8.dp)).clickable { showPeople = true }.padding(8.dp))
            }

            when {
                followed.isEmpty() -> EmptyCard(onScan = onScan, onShowInvite = { Endpoint.expectOffer(); showInvite = true })
                subject == null    -> ChooseCard(followed) { Prefs.setGraphSubject(context, it.key); tick++ }
                readings.isEmpty() -> WaitingCard(subject)
                else               -> {
                    HeroCard(context, readings, treatments, target, tempTarget, low, high)
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
                                    " · this phone" +
                                    // SAYS SO WHEN IT CANNOT DRAW THE BASAL.
                                    // An absent trace with no explanation reads
                                    // as "they had no basal", which is a
                                    // clinical claim this phone has no business
                                    // making on their behalf.
                                    (if (basalUnknown) " · no profile yet — basal not shown" else ""),
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

        // **INSIDE THE THEME, AND THIS IS NOT COSMETIC.** These used to sit
        // after the `MaterialTheme` block closed, so every dialog rendered
        // against Material's DEFAULT light scheme while the labels inside
        // them are hardcoded near-white for a dark one. The result was
        // white text on a white sheet: "Scan an invite", "Paste an invite",
        // "Units" and the rest were all present, all tappable, and all
        // invisible — the settings looked like an empty list titled
        // "People you follow".
        if (showInvite) InviteDialog(Endpoint.invite(context)) { showInvite = false }
        if (showPaste) PasteInviteDialog(
            onFollow = { Invites.queue(context, it); Sync.now(context); tick++ },
            onClose = { showPaste = false }
        )
        if (showCarrying) CarryingDialog(carrying, Prefs.carryingAt(context)) { showCarrying = false }
        if (showPeople) PeopleSheet(
            followed = followed,
            onScan = { showPeople = false; onScan() },
            onPaste = { showPeople = false; showPaste = true },
            onShowInvite = { showPeople = false; Endpoint.expectOffer(); showInvite = true },
            onChoose = { Prefs.setGraphSubject(context, it.key); tick++; showPeople = false },
            onUnits = { Prefs.setMmol(context, !Prefs.mmol(context)); tick++ },
            // The endpoint has to come back for this: see [Endpoint.restart].
            onKeysVault = {
                Prefs.setKeysVault(context, !Prefs.keysVault(context))
                // Turning the vault off must take the keys-only mode with it, or
                // the app would be left reading nothing at all.
                if (!Prefs.keysVault(context)) Prefs.setKeysOnly(context, false)
                Endpoint.restart(context)
                tick++
            },
            keysVault = Prefs.keysVault(context),
            onKeysOnly = { Prefs.setKeysOnly(context, !Prefs.keysOnly(context)); tick++ },
            keysOnly = Prefs.keysOnly(context),
            // Applied immediately rather than at next launch: a switch that takes
            // effect later is the defect this app already had once.
            onStayReachable = {
                Prefs.setStayReachable(context, !Prefs.stayReachable(context))
                // Applied now, not at next launch. Starting the service IS the
                // setting — there is nothing else it does.
                StayAwake.apply(context)
                tick++
            },
            stayReachable = Prefs.stayReachable(context),
            // Read every tick rather than remembered: the user grants this in
            // Android's settings, outside this app, and comes back expecting the
            // row to have noticed.
            dozeExempt = StayAwake.exemptFromDoze(context),
            onFixDoze = { StayAwake.ask(context) },
            onBand = { Prefs.cycleBand(context); tick++ },
            bandLabel = Prefs.bandLabel(context),
            mmol = Prefs.mmol(context),
            onClose = { showPeople = false }
        )
    }
}

@Composable
private fun HeroCard(
    context: android.content.Context,
    readings: List<Follower.Reading>,
    treatments: List<Follower.Treatment>,
    target: Pair<Double, Double>?,
    tempTarget: Follower.TempTarget?,
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
                // **SAYS WHEN IT IS TEMPORARY, AND WHY.** The band was always
                // the profile's, so a subject running an exercise target had
                // their profile band shown as theirs while the loop aimed
                // somewhere else. Now it follows the temporary one — and says
                // so, because a band that silently changes reads as somebody
                // editing their profile rather than going for a run.
                val label = "target ${Prefs.show(context, lo)}–${Prefs.show(context, hi)} " +
                    Prefs.unitLabel(context)
                Text(
                    tempTarget?.let { tt ->
                        val why = tt.why.lowercase().replace('_', ' ').takeIf { it.isNotBlank() && it != "custom" }
                        "$label · temporary${why?.let { ", $it" } ?: ""}, ${Follower.untilWords(tt.until)}"
                    } ?: label,
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
                Text(s.label, color = Good, fontSize = 15.sp,
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
            Text("Following ${subject.label}", color = Text1, fontSize = 17.sp)
            Text(
                "Nothing readable yet. Following costs them nothing and grants you nothing — " +
                    "they still have to share with you from their own phone.",
                color = Text2, fontSize = 14.sp
            )
        }
    }
}
