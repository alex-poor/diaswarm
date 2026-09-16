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

## Recommendation on bumping

**Bump — but not tonight, and not to a moving target.**

The case for bumping is strong: the pinned build has a visibly broken settings
screen and a permission whose stated justification is false. Both are exactly
what a tester checks, and both are fixed.

The case for waiting one day is also real: today's diff is large and hours old,
and **the rotation fix has never had a successful rotation run through it** —
the first is due ~15:34 on 2026-09-17.

**So: verify that rotation, then re-pin to that commit as 0.1.7 (codes 81/82).**
Bumping to a commit where everything shipped has been exercised at least once is
worth one day of waiting, and re-pinning mid-review costs them a pipeline run
either way — better to spend it once, on something finished.
