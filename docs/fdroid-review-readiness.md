# What an F-Droid tester will find — 2026-09-16

Against the checklist at
`gitlab.com/fdroid/wiki/-/wikis/Internal/Reviewing-new-apps`. MR !48469 is
pinned to **2e55f2b (0.1.6)**, so that commit is what gets tested — not HEAD.

## 🔴 Two things in the pinned build would likely fail tester review

### 1. Every dialog is white text on a white sheet

> *"The app can start and work normally."*

At 2e55f2b the dialogs are declared **after** the `MaterialTheme` block closes
(theme at line 91, `PeopleSheet` at line 149), so they render against Material's
default **light** scheme while every label inside is hardcoded near-white for a
dark one. *Scan an invite*, *Show my invite*, *Units*, both vault toggles — all
present, all tappable, all invisible.

**The settings menu is the first thing a tester opens.** They will see a dialog
titled "People you follow" containing nothing but explanatory grey paragraphs.
Found 2026-09-16 by the owner doing exactly that; fixed in `7c62da4`.

### 2. CAMERA is effectively required — and the manifest says it is not

> *"Check if the app can be used without granting optional runtime
> permissions."*

`AndroidManifest.xml` has said since 2026-09-11:

```xml
<!-- Scanning an invite. Optional: an invite can be pasted as text. -->
<uses-permission android:name="android.permission.CAMERA" />
```

**There was no paste path until `c693184`, today.** At the pinned commit the QR
scanner is the only way to follow anyone, so denying CAMERA leaves the app
unable to do the one thing it does — while the permission's own justification,
in the file a reviewer reads to check justifications, claims otherwise.

## 🟠 One that is inherent and needs pre-empting, not fixing

**A tester cannot exercise any described function.** Ayni shows somebody else's
glucose; it needs a publisher to grant it, and the publisher is the AAPS add-on
which [is deliberately not distributed](../fdroid/nz.diaswarm.ayni.yml). A
reviewer installs it, sees *"Not following anyone yet"*, and stops.

That is not a defect and the description says so plainly — but it will stall the
tester checklist at the first two boxes. **Offer a concrete way through in the MR
thread**: a synthetic test subject they can follow for the review period, from
`tools/mkfixture.py` data rather than anyone's real readings.

⚠️ **Do not offer to grant a reviewer from the loop phone.** That is a stranger
reading a real person's glucose, and no review convenience is worth it.

## 🟠 The relay will show up on PCAPdroid

> *"Check if the app connects to any web services on start. Such connections
> means NonFreeNet and TetheredNet may apply."*

It connects to an iroh relay on start — the default is n0's, in Singapore. A
tester running a network monitor will see it before anything else.

The description already explains it well ("carries encrypted traffic it cannot
read… chosen by the person sharing and travels in their invite, so it can be one
they run"). **Say two more things in the MR thread**: the relay implementation is
free software (iroh, Apache-2.0/MIT), and the relay is per-invite rather than
hardcoded — so neither NonFreeNet nor TetheredNet fits.

## 🟡 Likely questions, with answers ready

* **`FOREGROUND_SERVICE_SPECIAL_USE` rather than `dataSync`** — because
  `dataSync` is capped at ~6 h/24 h and this has to survive a night. The manifest
  comment says so; be ready to say it again.
* **`CHANGE_WIFI_MULTICAST_STATE`** — mDNS peer discovery on a LAN; p2panda
  cannot take the multicast lock itself.
* **Categories** — Health Manager + Sports & Health, already agreed on the MR.
* **Icon** — adaptive XML in the APK, with a 512px PNG in
  `fastlane/…/en-US/images/` because F-Droid's indexer will not use the XML.

## Recommendation on bumping — SUPERSEDED, and the bump is done

**Re-pinned to 0.1.7 (codes 81/82) on 2026-09-17, without the overnight run the
section below asked for.** That was a deliberate trade, so the reasoning it
replaces is left underneath rather than deleted.

What changed: the merge window is closing, and the two 🔴 defects above are in
the commit a tester would open first. Shipping a build whose settings screen
renders blank and whose CAMERA justification is false, in order to buy one
night of soak, is the worse of the two risks.

What the bump carries beyond those two fixes:

* `e9ab0d8` — the demo subject, which answers the 🟠 above: a tester can follow
  a synthetic publisher instead of nobody, with no real glucose involved.
* `3792fd6`, `063c1c7` — the follower's read path stops re-fetching and
  re-decrypting on every refresh. 63.1% of a core to 38.0% on phone B, same
  protocol either side. A battery fix for whoever installs this, not just for us.

What it does NOT carry, and should be said plainly on the MR if asked: **no
overnight run.** 0.1.7 has run on a phone — installed from a clean tree at
`18ec189`, live for minutes, reading and merging both vaults without a crash —
which is the bar `fdroid/nz.diaswarm.ayni.yml` sets before a tag ("do not tag a
version until it has actually run on a phone"). It is not the bar the section
below wanted, and the difference is a night.

Also not verified the way it should have been: the dialog fix was confirmed
**structurally** — `MaterialTheme` opens at FollowerApp.kt:94 and all four
dialogs now sit inside it — rather than by opening them on the device.
`adb input` events stopped reaching the app on phone B, tap and swipe alike, so
the screen could not be driven. The defect was structural and the structure is
fixed; somebody should still open that menu by hand before trusting it.

---

<details><summary>The superseded recommendation, kept because it was right about
the trade it was making</summary>


**Bump — but not tonight, and not to a moving target.**

The case for bumping is strong: the pinned build has a visibly broken settings
screen and a permission whose stated justification is false. Both are exactly
what a tester checks, and both are fixed.

✅ **THE ROTATION FIX NO LONGER NEEDS A DEVICE TO VERIFY.** The publish lived
inside a JNI function, and a `JNIEnv` cannot be built in a unit test — which is
precisely why it shipped broken: the only way to exercise it was to install the
plugin and wait a day. `rotate_and_publish` is now that glue with the `JNIEnv`
taken off the front, and `a_rotation_puts_its_update_in_the_control_log` asserts
the thing the outage actually was — **not "did it rotate", because the broken
version rotated perfectly, but "did anybody get told"**. Confirmed to fail
against the original code (`the control log went 1 -> 1`) and pass against the
fix.

So the reason to wait has largely gone. What is left of it: today's diff is
large and hours old, and nothing has run overnight.

**So: let it run overnight, then re-pin to that commit as 0.1.7 (codes 81/82).**
The remaining reason to wait is not the rotation any more — it is that a large
diff written in one evening deserves one night of the daemon and both phones
running before it is handed to somebody else to test. Re-pinning mid-review
costs them a pipeline run either way; better to spend it once, on something that
has survived a night.

</details>
