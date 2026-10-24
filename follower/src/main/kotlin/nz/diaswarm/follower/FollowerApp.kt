package nz.diaswarm.follower

import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.GroupAdd
import androidx.compose.material.icons.outlined.Person
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
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.withContext

// Local names for the shared tokens (see Theme.kt), so the composables below
// read unchanged while the palette lives in one place.
private val Bg = AyniColor.Bg
private val Card = AyniColor.Card
private val Text1 = AyniColor.Text1
private val Text2 = AyniColor.Text2
private val Good = AyniColor.InRange
private val Insulin = AyniColor.Insulin
private val Carbs = AyniColor.Carbs

@Composable
fun FollowerApp(onScan: () -> Unit, scanned: String?, onScanHandled: () -> Unit) {
    val context = LocalContext.current
    var tick by remember { mutableStateOf(0) }
    var showInvite by remember { mutableStateOf(false) }
    var showPeople by remember { mutableStateOf(false) }
    var showPaste by remember { mutableStateOf(false) }
    var showCarrying by remember { mutableStateOf(false) }
    // When the on-screen sync loop last asked for a pass, so the waiting state
    // can say "last tried 20s ago" rather than leave someone staring at a card
    // with no sign anything is happening.
    var lastSyncAt by remember { mutableStateOf(System.currentTimeMillis()) }

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
            lastSyncAt = System.currentTimeMillis()
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

    // **EVERY ONE OF THESE READS BOTH VAULTS OVER JNI, AND THEY USED TO RUN ON
    // THE MAIN THREAD.** They were a stack of `remember(tick)` blocks, so each
    // fired during composition — on the UI thread — and the clock bumps `tick`
    // every fifteen seconds. As a subject's history grew, the merge of core and
    // keys crossed the frame budget and then the ANR budget: the activity took
    // eight seconds to first paint and hung outright when the background sync
    // pass held the vault lock at the same moment. Found on phone B, reproduced
    // with the alert work stashed, so it predated it.
    //
    // Now one bundle, loaded off the main thread. `produceState` relaunches the
    // load when `tick` changes and keeps the PREVIOUS bundle on screen until the
    // new one is ready, so a refresh never blanks the hero and never blocks a
    // tap. The first load (null) shows a spinner — which the splash usually
    // still covers anyway.
    val data by produceState<AppData?>(initialValue = null, tick) {
        value = withContext(Dispatchers.IO) { loadAppData(context) }
    }
    val loading = data == null
    val subject = data?.subject
    val readings = data?.readings ?: emptyList()
    val treatments = data?.treatments ?: emptyList()
    val target = data?.target
    val tempTarget = data?.tempTarget
    val followed = data?.followed ?: emptyList()
    val carrying = data?.carrying ?: -1
    val low = data?.low ?: Prefs.DEFAULT_LOW_MGDL
    val high = data?.high ?: Prefs.DEFAULT_HIGH_MGDL
    val scheduled = data?.scheduled
    val basal = data?.basal ?: emptyList()
    val basalUnknown = data?.basalUnknown ?: false

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
                // First load only — the bundle is still being read off-thread.
                // After that, the previous bundle stays on screen through a
                // refresh, so this never flashes between updates.
                loading -> Box(Modifier.fillMaxWidth().weight(1f), contentAlignment = Alignment.Center) {
                    CircularProgressIndicator(color = Good)
                }
                followed.isEmpty() -> EmptyCard(onScan = onScan, onShowInvite = { Endpoint.expectOffer(); showInvite = true })
                subject == null    -> ChooseCard(followed) { Prefs.setGraphSubject(context, it.key); tick++ }
                readings.isEmpty() -> WaitingCard(subject, lastSyncAt)
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
        if (showPeople) SettingsScreen(
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
            alertOn = { Alerts.enabled(context, it) },
            // Unit-correct, so the thresholds read in whatever the units toggle
            // shows. The fixed two state the clinical line; the band two point at
            // the viewer's own low/high line, which is what they actually fire on.
            alertSubtitle = { level ->
                val unit = Prefs.unitLabel(context)
                when (level) {
                    Alerts.Level.URGENT_LOW -> "At or below ${Prefs.show(context, Glucose.URGENT_LOW_MGDL)} $unit"
                    Alerts.Level.LOW -> "Below your low line, ${Prefs.show(context, Prefs.lowLine(context))} $unit"
                    Alerts.Level.HIGH -> "Above your high line, ${Prefs.show(context, Prefs.highLine(context))} $unit"
                    Alerts.Level.VERY_HIGH -> "At or above ${Prefs.show(context, Glucose.VERY_HIGH_MGDL)} $unit"
                }
            },
            onToggleAlert = { level -> Alerts.setEnabled(context, level, !Alerts.enabled(context, level)); tick++ },
            notificationsBlocked = notificationsBlocked(context),
            onFixNotifications = { openNotificationSettings(context) },
            onClose = { showPeople = false }
        )
    }
}

/**
 * Everything the one screen draws, read in a single off-thread pass.
 *
 * Bundled so the whole screen comes from one consistent snapshot — the hero, the
 * chart and the band all describe the same moment — and so there is exactly one
 * place that touches the vault per refresh rather than a dozen scattered
 * `remember` blocks each opening it again.
 */
private data class AppData(
    val subject: Follower.Subject?,
    val readings: List<Follower.Reading>,
    val treatments: List<Follower.Treatment>,
    val target: Pair<Double, Double>?,
    val tempTarget: Follower.TempTarget?,
    val followed: List<Follower.Subject>,
    val carrying: Int,
    val low: Double,
    val high: Double,
    val scheduled: Double?,
    val basal: List<Follower.BasalStep>,
    val basalUnknown: Boolean
)

/** Read the whole screen's data. Always called off the main thread — see [FollowerApp]. */
private fun loadAppData(context: android.content.Context): AppData {
    val subject = Follower.chosen(context)
    val hours = Prefs.rangeHours(context)
    val readings = subject?.let { Follower.readings(context, it, hours) } ?: emptyList()
    val treatments = subject?.let { Follower.treatments(context, it, hours) } ?: emptyList()
    val scheduled = subject?.let { Follower.scheduledBasalOrNull(context, it, System.currentTimeMillis()) }
    val basal = if (readings.isEmpty()) emptyList()
        else Follower.basalSteps(treatments, scheduled, readings.first().at, readings.last().at)
    return AppData(
        subject = subject,
        readings = readings,
        treatments = treatments,
        target = subject?.let { Follower.target(context, it) },
        // Separate from target so the card can SAY it is temporary; a band that
        // silently changes looks like they edited their profile.
        tempTarget = subject?.let { Follower.runningTempTarget(context, it) },
        followed = Follower.following(context),
        carrying = Prefs.carrying(context),
        low = Prefs.lowLine(context),
        high = Prefs.highLine(context),
        // NULLABLE ON PURPOSE — see [Follower.scheduledBasalOrNull]. Null is "no
        // profile for them", not a basal of zero, and the chart must not confuse
        // the two.
        scheduled = scheduled,
        basal = basal,
        basalUnknown = scheduled == null && treatments.any { it.kind == "tbr" && !it.absolute }
    )
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
    // Old data wears no confident colour: the number greys and the age turns
    // into a warning. This is the whole point of the app said in the hero.
    val stale = Glucose.isStale(last.at)

    Card(colors = CardDefaults.cardColors(containerColor = Card), modifier = Modifier.fillMaxWidth()) {
        Column(Modifier.padding(18.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
            // AGE FIRST AND ALWAYS. Never a bare number.
            Text(
                (if (stale) "⚠ " else "") + Follower.ageWords(last.at),
                color = if (stale) AyniColor.Warn else Text2, fontSize = 13.sp,
                modifier = Modifier.fillMaxWidth(), textAlign = androidx.compose.ui.text.style.TextAlign.End
            )
            // THE SAME RULE AS THE CHART, from the same function. These
            // disagreed once — the number here was unconditionally green while
            // the identical reading was drawn red below it, because the chart
            // classified against a single-point target and this did not
            // classify at all.
            val tone = Glucose.color(last.mgdl, lowLine, highLine, stale)
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
                    .background(if (on) AyniColor.CardRaised else Color.Transparent)
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
            // The swarm's own green, so the first screen reads as this app and not
            // a default Material form.
            Icon(Icons.Outlined.GroupAdd, contentDescription = null, tint = Good,
                modifier = Modifier.size(32.dp))
            Text("Not following anyone yet.", color = Text1, fontSize = 17.sp, fontWeight = FontWeight.SemiBold)
            Text(
                "Ask them to show you their invite, then scan it. They decide what " +
                    "you can read, and can stop at any time.",
                color = Text2, fontSize = 14.sp
            )
            // The primary way in is filled in the brand green; showing your own
            // invite is the quieter second path.
            Button(
                onClick = onScan, modifier = Modifier.fillMaxWidth(),
                shape = RoundedCornerShape(AyniRadius.Pill),
                colors = ButtonDefaults.buttonColors(containerColor = Good, contentColor = Bg)
            ) { Text("Scan their invite", fontWeight = FontWeight.SemiBold) }
            OutlinedButton(
                onClick = onShowInvite, modifier = Modifier.fillMaxWidth(),
                shape = RoundedCornerShape(AyniRadius.Pill),
                border = BorderStroke(1.dp, AyniColor.Line),
                colors = ButtonDefaults.outlinedButtonColors(contentColor = Text1)
            ) { Text("Show my invite") }
        }
    }
}

@Composable
private fun ChooseCard(followed: List<Follower.Subject>, onChoose: (Follower.Subject) -> Unit) {
    Card(colors = CardDefaults.cardColors(containerColor = Card), modifier = Modifier.fillMaxWidth()) {
        Column(Modifier.padding(vertical = 6.dp)) {
            // Picking one at random would be picking a PERSON at random.
            Text("Whose glucose should this show?", color = Text1, fontSize = 17.sp,
                fontWeight = FontWeight.SemiBold,
                modifier = Modifier.padding(start = 18.dp, end = 18.dp, top = 12.dp, bottom = 4.dp))
            followed.forEachIndexed { i, s ->
                SubjectRow(
                    s.label,
                    if (s.reached) "holding their history" else "nothing has arrived yet",
                    selected = false,
                    leading = Icons.Outlined.Person
                ) { onChoose(s) }
                if (i < followed.lastIndex) RowDivider()
            }
        }
    }
}

/**
 * The pending handshake, said honestly.
 *
 * Following is two-party: this phone asks, and the other phone has to share
 * back. The old card said "nothing readable yet" and stopped — so a person
 * could not tell whether they had done it wrong or were simply waiting on
 * someone else. Now it names who it is waiting on, says whose move it is, shows
 * that it is still trying, and offers to nudge them.
 *
 * [subject.reached] splits the two states that look alike and are not: nothing
 * has arrived at all, versus their records are held here but no glucose is
 * readable yet.
 */
@Composable
private fun WaitingCard(subject: Follower.Subject, lastSyncAt: Long) {
    val context = LocalContext.current
    Card(colors = CardDefaults.cardColors(containerColor = Card), modifier = Modifier.fillMaxWidth()) {
        Column(Modifier.padding(18.dp), verticalArrangement = Arrangement.spacedBy(14.dp)) {
            Text("Waiting on ${subject.label}", color = Text1, fontSize = 17.sp, fontWeight = FontWeight.SemiBold)
            Text(
                if (!subject.reached)
                    "You've asked to follow them. Now they share with you from their own phone — " +
                        "until they do, there is nothing here to read. Following costs them nothing."
                else
                    "This phone is holding their records, but no glucose is readable yet — they may not " +
                        "be sharing readings, or it is still catching up.",
                color = Text2, fontSize = 14.sp
            )
            StatusBanner(
                BannerTone.Wait,
                "Checking every ${SyncWorker.EVERY_SECONDS / 60} min · last tried ${Follower.ageWords(lastSyncAt)}"
            )
            if (!subject.reached) {
                Text(
                    "Remind them to share  ›",
                    color = Insulin, fontSize = 14.sp,
                    modifier = Modifier.clip(RoundedCornerShape(8.dp))
                        .clickable { shareReminder(context) }.padding(vertical = 4.dp)
                )
            }
        }
    }
}

/**
 * Whether glucose alerts cannot reach this phone because notifications are off.
 *
 * Only on 33+, where the grant is a runtime permission somebody can refuse or
 * revoke; below it, notifications are allowed at install and this is never true.
 */
private fun notificationsBlocked(context: android.content.Context): Boolean =
    android.os.Build.VERSION.SDK_INT >= android.os.Build.VERSION_CODES.TIRAMISU &&
        context.checkSelfPermission(android.Manifest.permission.POST_NOTIFICATIONS) !=
        android.content.pm.PackageManager.PERMISSION_GRANTED

/** Open this app's notification settings, where the grant is reconsidered. */
private fun openNotificationSettings(context: android.content.Context) {
    val intent = android.content.Intent(android.provider.Settings.ACTION_APP_NOTIFICATION_SETTINGS)
        .putExtra(android.provider.Settings.EXTRA_APP_PACKAGE, context.packageName)
        .addFlags(android.content.Intent.FLAG_ACTIVITY_NEW_TASK)
    try {
        context.startActivity(intent)
    } catch (e: Throwable) {
        android.util.Log.w(SyncWorker.TAG, "no notification settings screen: $e")
    }
}

/** Open the system share sheet with a prewritten nudge for the person being followed. */
private fun shareReminder(context: android.content.Context) {
    val msg = "I'm following your glucose with Ayni, but nothing reaches me until you share " +
        "with me from your looping app. When you get a moment?"
    val send = android.content.Intent(android.content.Intent.ACTION_SEND).apply {
        type = "text/plain"
        putExtra(android.content.Intent.EXTRA_TEXT, msg)
    }
    context.startActivity(
        android.content.Intent.createChooser(send, "Remind them")
            .addFlags(android.content.Intent.FLAG_ACTIVITY_NEW_TASK)
    )
}
