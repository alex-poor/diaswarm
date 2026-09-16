# What each role means for the apps

**Companion to [roles.md](roles.md).** That document says who the actors are and
what each may truthfully be told. This one says what has to change in the
software, per role, and what it implies for a screen.

**Every "today" below was read from the code, not remembered.** Where something
already exists it is marked so, because the useful output of this exercise is the
*delta*, and a list that re-proposes what is already built is worse than no list.

---

## The finding that came out of doing this — now fixed

✅ **All three of the small changes below shipped on 2026-09-16**, along with the
subject handles. What follows is kept as written because the reasoning is still
the reasoning; the status lines say what is done.

⚠️ **A scoped grant is meaningless without a rotation schedule, and nothing
rotated.** *(Fixed: `rotateIfDue`, daily, commit `8988329`.)*

`Vault::grant_since` filters by *when a secret was minted*. Secrets are minted
per group operation — a grant, a withdrawal, an explicit `rotate`. A subject who
never rotates holds one secret covering everything, so every `since` either
hands over all of it or none.

**So "share the last 90 days" is a scheduling feature before it is a UI
feature.** The subject's app has to rotate on a cadence, and the cadence sets the
finest scope any grant can express: rotate daily and windows land to the day;
rotate never and the control does nothing.

**The cost is known and small.** D26 measured it: a year of daily rotation is a
366-secret bundle and a **0.76 ms** welcome. There is no performance argument
against daily.

This is a product change nobody had written down, and it blocks both the
clinician and the tightened family grant.

---

## 1. Subject — the AAPS plugin

**Today.** Preference screen with: show invite (QR), *Grant reader* and *Revoke
reader* as **raw string inputs**, and *Show readers* listing who is granted.
Publishing, sealing and carrying all work.

**Changes**

| change | why | size |
|---|---|---|
| ✅ **Rotate on a schedule** | the finding above — without it no grant can be scoped | done, `8988329` |
| ✅ **Scope picker on grant** | D11 says family needs 24 h; they got everything | done, `cbfc379` |
| ✅ **Show each reader's scope** in *Show readers* | a grant list that omits scope cannot answer "what did I share?" | done, seen on phone B |
| ✅ **Honest withdrawal copy** | the wording is unwritten, so it will be invented at the worst moment | done, seen on phone B |
| ✅ **A name for the subject** | a follower showed sixteen hex characters | done, `a1d51af` + `cd3f4ac` |

**UI implications**

* ✅ **The window now exists**, above the grant control: everything, a day, a
  week, 30 days, 90 days. It defaults to everything, because that is what the
  app did before and narrowing silently would take history from people already
  relying on it. **Choosing a person is still a text field** — the two are one
  act and only half of it has a screen.
* **Withdrawal needs a confirmation that tells the truth**, and the truth is
  oddly specific: *"They will lose today and keep every day that already
  finished."* Not "revoked", not "access removed". See roles.md — this bites
  back exactly one day because `seal` re-seals the accumulated day under the
  latest secret.
* **The readers list is the grant log made visible**, which is the substitute
  D13 offers for a read log. It should say so, because a user who thinks it is
  a read log has been misled by omission.

---

## 2. Family reader — Ayni

**Today.** Scan invite, show invite, pick whose glucose to show, a graph with a
range picker, and **reading age already on screen** (`Follower.ageWords`) —
which is the single most important thing this role needs and it is already
right.

**Changes**

| change | why | size |
|---|---|---|
| ✅ **Show that they are carrying, and how much** | D30 makes carrying the price of reading, and it happened only in the log | done, `b5d985b` |
| ✅ **Show the subject's name** | "Following 9eeeac47cd164df7" | done, `cd3f4ac` |
| **Nothing else** | the reading, its age and the trend are already there | — |

**UI implications**

* **Carrying needs a home on the main screen, not in settings.** D30 is the
  principle the app is named after, and an invariant the user never sees is one
  they cannot consent to. It does not need to be prominent — a line, a number —
  but it needs to be visible without going looking.
* ⚠️ **Do not offer a way to read without carrying.** The preference exists
  internally (`keysOnly`) and must not become a user-facing "just let me watch"
  toggle. That would be selling the thing D30 decided not to sell.

---

## 3. Carrier — Ayni and the desktop peer

**Today.** Ayni carries (`keysVault`, `keysOnly` preferences) and the desktop
peer carries by default. **Neither surfaces what is held.**

**Changes**

| change | why | size |
|---|---|---|
| **A "what you hold" view**: subjects carried, bytes on disk | roles.md: they must see the cost | small on Ayni, needs a UI on the peer |
| **Say plainly it cannot be read** | structural, not policy — a carrier has no grant and no secret | copy only |
| **A stop control, and what stopping costs** | consent that cannot be withdrawn is not consent | small |

**UI implications**

* **The honest framing is "a carrier breach is not a data breach"**
  (`feasibility.md` §11, claim 3). That is the sentence that makes someone
  willing to carry strangers' bytes, and it is true.
* ⚠️ **Never say "your data is stored across a distributed network"** while the
  pool is three devices. True only at scale, and this is exactly the claim that
  ages into a lie without anyone editing it.

---

## 4. Clinician — does not exist

**Today.** The protocol can do this as of 2026-09-16 (`grant_since` +
`scoped_grant.rs`). There is no application.

**Changes.** This is D29 step 3 and it is a product, not a change. What it needs
from the roles work:

* The window granted, displayed **in the subject's words**, on the clinician's
  screen — so both ends describe the same thing the same way.
* It must be *unreadable*, not hidden, outside that window. The test asserts
  this (`Skipped::not_ours`), and the UI should be able to say it.

**UI implications**

* ⚠️ **The screen may not offer a window the grant does not enforce.** This is
  the specific harm D29 exists to prevent, and it is now avoidable rather than
  merely warned about.
* ⚠️ **"90 days" is only honest with a rotation schedule.** Both halves ship
  together or the screen lies.

---

## 5. Researcher — does not exist

**Today.** Nothing. D5 settled the shape: an ordinary granted peer on the swarm
side, CSV / Parquet / OPEN shapes on the other.

**Changes.** Out of scope until the clinician path exists — the two are the same
screen at different scopes, and building the general case first is how the
specific one gets it wrong.

**UI implications**

* ❌ **Never imply donation is revocable the way a family grant is.** Once
  aggregated and exported it is beyond reach. `rights.md` §6 marks Benefit a
  **fail on intent only**; that changes when this ships and not before.

---

## Order, and why

1. ✅ **Rotation on a schedule** (subject) — `8988329`.
2. ✅ **Carrying, visible** (Ayni) — `b5d985b`.
3. ✅ **Scope picker on grant** (subject) — `cbfc379`.
4. ⬜ **Desktop peer UI** (carrier + subject + clinician). D29 step 1, and three
   of five roles differ only in what they are granted, not in what they run.
5. ⬜ **The clinician gateway.** D29 step 3, honest for the first time.

⚠️ **1–3 are built but not installed on either phone**, and the loop phone's
install is the owner's call. Built is not shipped, and this project has been
caught by that distinction before.

✅ **Both small ones from the subject table are done and were opened on phone B**
(AAPS installed there alongside Ayni, loop disabled, virtual pump). The readers
list carries each reader's scope and the withdrawal copy says the true sentence
from roles.md — "they lose today and keep every day that already finished".

**Looking at it changed it.** The first build's unknown-scope line ran to two
lines under every reader and buried the key and purpose it sits between; it is
now one line, "Scope not recorded — shared before this was kept". Nothing but
opening the dialog would have shown that.

**The scope shown is the WIDEST grant ever made, not the latest**, because
`grant_since` narrows the next grant and takes nothing back — a reader given
everything on Monday and re-granted a day on Friday still holds everything.
Showing Friday's number would answer "what did I share?" with a comfortable lie.
`reader_scope_widens_and_never_narrows` in `crates/diaswarm-core/tests/vault.rs`
is what holds that. A reader granted before the field existed reads as *not
recorded*, which is deliberately not the same as *everything*.

⚠️ **1–3 are small and none of them is a new product.** The reason to do them
first is not that they are easy: it is that each one closes a gap between
something already decided and something a person can actually see or choose.
