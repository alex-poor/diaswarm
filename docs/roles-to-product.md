# What each role means for the apps

**Companion to [roles.md](roles.md).** That document says who the actors are and
what each may truthfully be told. This one says what has to change in the
software, per role, and what it implies for a screen.

**Every "today" below was read from the code, not remembered.** Where something
already exists it is marked so, because the useful output of this exercise is the
*delta*, and a list that re-proposes what is already built is worse than no list.

---

## The finding that came out of doing this

⚠️ **A scoped grant is meaningless without a rotation schedule, and nothing
rotates today.**

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
| **Rotate on a schedule** | the finding above — without it no grant can be scoped | small, needs a preference |
| **Scope picker on grant** | D11 says family needs 24 h; today they get everything | small, `grant_since` exists |
| **Show each reader's scope** in *Show readers* | a grant list that omits scope cannot answer "what did I share?" | small |
| **Honest withdrawal copy** | see below — the current wording is unwritten, so it will be invented at the worst moment | copy only |

**UI implications**

* **Grant stops being a text field.** Choosing a person and choosing a window
  are one act. The window is the interesting half and currently has no
  representation at all.
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
| **Show that they are carrying, and how much** | D30 makes carrying the price of reading; roles.md says they must see it. Nothing on screen mentions it today | small |
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

1. **Rotation on a schedule** (subject). Unblocks everything else about scope,
   costs 0.76 ms of welcome per year of history, and is invisible to every user
   until something uses it.
2. **Carrying, visible** (Ayni). Smallest gap between a settled principle and
   what a person can see. D30 is the project's name.
3. **Scope picker on grant** (subject). Closes the over-granting debt roles.md
   identifies — the difference between a withdrawal that leaves someone a day
   and one that leaves them a life.
4. **Desktop peer UI** (carrier + subject + clinician). D29 step 1, and three of
   five roles differ only in what they are granted, not in what they run.
5. **The clinician gateway.** D29 step 3, honest for the first time.

⚠️ **1–3 are small and none of them is a new product.** The reason to do them
first is not that they are easy: it is that each one closes a gap between
something already decided and something a person can actually see or choose.
