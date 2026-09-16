# Roles: who does what, and what each may truthfully be told

**This exists because the question was asked and never answered in one place.**
*"What are the use cases, who does what, what do they need to see/know, and how
can they be served best?"* The answers were scattered across D5, D11, D13, D15,
D29 and D30, and both remaining pieces of work — the desktop peer's UI and the
clinician gateway — are questions about **what a screen shows whom**. Neither can
be designed from a decision index.

**It is not a new design.** Every constraint below is already settled somewhere,
and cited. What is new is putting the actors side by side, because that is the
view that shows which promises a UI is allowed to make.

**For what this means for the apps, see
[roles-to-product.md](roles-to-product.md)** — the per-role deltas, read from
the code rather than remembered, and the finding that fell out of doing it: a
scoped grant is meaningless without a rotation schedule, and nothing rotates.

**The honesty rules are load-bearing, not garnish.** `feasibility.md` §11 lists
sentences this project may never say. A role's "must never be told" column is
that list, applied to the person in front of the screen.

---

## The five roles at a glance

| role | reads plaintext? | carries ciphertext? | exists today? |
|---|---|---|---|
| **Subject** | their own | their own | ✅ AAPS plugin |
| **Family reader** | yes, by grant | **yes — required** | ✅ Ayni |
| **Carrier** | no | yes | ✅ Ayni, desktop peer |
| **Clinician** | yes, time-scoped | yes, by the same rule | ⚠️ protocol only |
| **Researcher** | aggregate, via gateway | n/a | ❌ not built |

⚠️ **Reader and carrier are not alternatives.** D30 makes carrying the price of
reading — *"if you READ data in any capacity/form/shape then you implicitly have
to opt in to carry it as well"*. Every row that reads is also a row that carries.
A UI offering "read without carrying" is offering something this project has
decided not to sell.

---

## 1. The subject

**The person with diabetes, whose phone seals and publishes.** Everyone else in
this document exists relative to them.

**What they do.** Seal readings into epoch segments, issue grants, withdraw
them, and carry their share of strangers' ciphertext like anyone else.

**What they must see**

* Every grant they have issued, to whom, for what purpose, and when.
* Every withdrawal.
* Every time data left the phone, and toward whom — D13's grant log is public,
  signed and tamper-evident, and that is the honest substitute for a read log.
* That their phone is the source of truth and needs no server (`feasibility.md`
  §11, claim 2).

**What they must never be told**

* ❌ *"We can show you who read your data."* Nobody can. Anyone may pull
  ciphertext and there is nothing to log.
* ❌ *"Delete my data."* Say *"nothing new is added and no new key is issued."*
* ❌ *"Revoke"* as though it recalls closed days. Say *"nothing new will be
  sent after you stop it, and they keep what they already have."*

  ⚠️ **It bites back further than "forward only", and by exactly one day.**
  `seal` re-seals the whole accumulated day under the *latest* secret, so a
  reader revoked at noon loses this morning as well — the segment they could
  already open is re-sealed out from under them. Closed days keep their own
  secret and are kept forever. So the true sentence is *"they lose today and
  keep every finished day"*, which is worth saying accurately because it is
  better than people expect and still not what they hope.
* ❌ Anything implying a follower's view is safe for a dosing decision. D7's
  read-only isolation is a safety property.

**Constrained by:** D7 (read-only), D13 (the grant log names nobody), D11.

---

## 2. The family reader

**The flagship, and the only role the project was built for.** A parent, a
partner. D11 settled this: *"the point is privacy and sovereignty — a person
shares their own history with friends, family and clinicians, revocably, with
nobody in the middle."*

**What they need is narrower than it looks: 24 hours.** That is what D11 asks
for and what Ayni shows. Everything about grant scope follows from that number
being small.

**What they must see**

* The current reading, **and its age**, always. A stale number presented as
  current is the dangerous failure for this role — not an error on screen.
* Enough history to read a trend.
* That they are carrying, and roughly how much (D30).

**What they must never be told**

* ❌ That this is a medical device, or that the view is suitable for dosing.
* ❌ *"Trustless."* They can screenshot and forward, and no protocol reaches
  that.

✅ **They can now be given a day.** `Vault::grant_since` narrows the bundle, the
subject's app rotates daily so a window has boundaries to land on, and a picker
above the grant control chooses between everything, a day, a week, 30 and 90
days.

⚠️ **It defaults to everything**, deliberately: that is what the app did before,
and narrowing silently would take history from people already relying on it. So
the over-granting is now a choice rather than a certainty — which is the most a
default can honestly be.

**Constrained by:** D11, D30, D15.

---

## 3. The carrier

**Holds bytes it cannot read, so that a sleeping phone is still readable.**
D15's promise — a subject whose phone is asleep stays available because somebody
else holds the ciphertext — is only true if carriers exist.

**What they do.** Carry a share of subjects they were never introduced to,
announce what they hold so others can find it, and adopt more when the pool
needs it.

**What they must see**

* How many subjects they carry, and how much space it costs.
* That they cannot read any of it, and that this is structural rather than a
  policy — a carrier has no grant and no secret.
* A way to stop, and what stopping costs the pool.

**What they must never be told**

* ❌ *"Your data is stored across a distributed network"* while there is
  effectively one carrier. True only at scale.
* ❌ That carrying is auditing, or that it gives any visibility into content.

⚠️ **A carrier breach is not a data breach**, and that is worth saying plainly
to whoever runs one (`feasibility.md` §11, claim 3).

**Constrained by:** D15, D28 (the pool carries keys logs), D30.

---

## 4. The clinician

**A time-bounded reader, and the role that was blocked until today.** D29 split
the desktop into two products precisely so this one could not be built on a
grant model that cannot express what its UI would promise.

**What they need.** A window — "the last 90 days" — not a history.

**What they must see**

* Exactly what window they were granted, in the same words the subject chose.
* That the window is what they hold, and older data is not merely hidden but
  unreadable to them.

**What they must never be told**

* ❌ A window the grant does not actually enforce. This is the specific failure
  D29 exists to prevent: *"a gateway screen offering 'share the last 90 days'
  that in fact hands over every day the subject has ever sealed."*

✅ **As of 2026-09-16 the protocol can keep that promise.**
`Vault::grant_since` narrows the bundle; `scoped_grant.rs` asserts the reader
opens the day after its cutoff and is refused the day before.

⚠️ **With two limits a UI must respect.** The scope is by *when the secret was
minted*, not by what it covers, so "90 days" is only true if the subject rotates
on a schedule (`Vault::rotate`). And it is not a revocation: a clinician already
holding an older secret keeps it.

**Constrained by:** D29, D26, `feasibility.md` §11.

---

## 5. The researcher

**Not a peer, and never will be.** D5 settled the shape: *"researchers will not
run a p2panda node, so the endpoint is an impedance match — an ordinary granted
peer on the swarm side, ordinary research formats on the other."*

**What they must see.** CSV, Parquet, the OPEN / OpenAPS Data Commons shapes.
Nothing about p2panda, topics, or epochs.

**What they must never be told**

* ❌ That donation is revocable in the sense the subject understands. Once
  aggregated and exported, withdrawal does not reach it. Say what it does reach.

❌ **Not built, and deliberately third.** D29's sequencing: desktop peer, then
scoped grants, then the gateway. Step 2 landed today; this is step 3, and
`rights.md` §6 already marks Benefit as a **fail on intent only** — the gateway
is how that stops being true.

**Constrained by:** D5, D11 (research is not the flagship), `rights.md` §6.

---

## Why a revocation cannot reach backwards

Asked directly, and worth recording because it is the first thing anyone
reasonable assumes: **can a withdrawal remove the history already shared?**

**No, for two independent reasons, either of which alone would be enough.**

1. **The secrets are on the reader's device.** A grant hands over `GroupSecret`s
   that live in their vault. `revoke` rotates the group secret so what comes
   next is sealed under one they do not hold. Nothing reaches into their
   storage, and nothing could.
2. **The ciphertext is permanent.** A DHT cannot unpublish
   (`feasibility.md` §7.4). Closed days are replicated across carriers — and
   under D30 the revoked reader is themselves one of those carriers.

**Crypto-shredding does not rescue it.** Destroying a key works when you hold
the only copy. Here the reader holds the key *and* the bytes.

**An honest client deleting on request is not a protocol property.** It works
against a cooperative reader and is worthless against any other, so it must
never be offered as a guarantee.

### What actually limits the damage: give less to begin with

The only real mitigation is scope at grant time. A family reader needs 24 hours
(D11) and currently receives every secret the subject holds — so a withdrawal
today leaves them holding everything. A reader granted through
`Vault::grant_since` against a daily rotation holds one day, so the same
withdrawal leaves them one day.

**That is the same over-granting debt this document identifies below**, arrived
at from the other direction. It is not a tidiness issue: it is the difference
between a withdrawal that leaves somebody a day and one that leaves them a life.

---

## What this makes obvious

1. **The family reader is over-granted.** They need 24 hours and receive
   everything. `grant_since` exists and has no caller. That is the smallest gap
   between what is decided and what is shipped.
2. **Every reading role is also a carrying role**, so there is no "viewer" UI to
   design that does not also answer "and here is what you hold".
3. ~~**The clinician and researcher screens are the same screen at different
   scopes**~~ — both are "a window, honestly described" — which is why D29 made
   the grant model the blocker rather than the UI.

   🔴 **WITHDRAWN 2026-09-16.** They differ in *shape*, not only scope: a
   clinician wants an AGP-style **digest in FHIR** (§10.7), a researcher wants
   rows. A Libre 3 reports every minute, so a 90-day window is ~143,000
   readings — a "scope" difference cannot turn that into something a clinic
   reads. The conclusion that follows — build the clinician first — still
   holds, but because the two are different products, not one generalised.
4. **Three of the five roles can be served by one desktop application.** Subject,
   carrier and clinician differ in what they are granted, not in what they run.
   That is the argument for D29 step 1 being one product.
