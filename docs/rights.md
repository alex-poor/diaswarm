# The Diabetes Data Rights Charter, and where this project fails it

[diabetesdatarights.com](https://www.diabetesdatarights.com/) is co-producing a
**Diabetes Data Rights Charter**: a set of principles that people with diabetes
(PwD) can hold stakeholders to. Core partners are Birmingham Law School,
University College Dublin and the Steno Diabetes Center; the content comes from
seven roundtables with 40+ people with diabetes across 16 countries during
2024–25, run under co-production rules giving participants equal voice. Launch
is anticipated in 2026.

**This document exists because a project that claims to give people control over
their diabetes data should be able to say, specifically, where it does not.**
[feasibility.md §11](feasibility.md) does that for the word *decentralised*.
This does it for the word *rights*, against an external standard written by the
people whose data it is rather than by the person who wrote the code.

Same rules as §11 and [decisions.md](decisions.md): each principle gets a
verdict, and each verdict gets the evidence that would change it. Two of the
verdicts below are **fail**, and one of those is not fixable.

---

## 0. What the Charter is, and who it is aimed at

The Charter's own statement of purpose: to *"articulate a set of principles that
key stakeholders should consider to enable PwD to, safely and securely, exercise
control over their diabetes data in a manner which best meets their personal
needs and values."*

**It is addressed to industry and health systems, not to builders.** The ask is
that device manufacturers and healthcare institutions change practice. The
project's own [exploratory review](https://www.diabetesdatarights.com/s/Patient-led-Data-Rights-Exploratory-Review-Final-Dec-2024.pdf)
(Dec 2024, surveying 33 comparable charters) is blunt that documents like this
carry essentially no legal force — only the state-linked ones do — and that they
work as advocacy leverage instead. It recommends the Charter adopt a blended
shape: overarching principles, each underpinned by specific rights.

**The banner phrase is PeSOS** — *person-specific open and secure access to
real-time data* — which grew out of a roundtable participant proposing the
definition *"open, secure, real-time access"*.

**The principle set has moved and is not final.** The website currently lists
eight: Access, Interoperability, Consent & Control, Privacy & Data Safety,
Benefit, Technology & Human Intervention, Trust & Transparency, Responsibility.
The [January 2025 virtual roundtables](https://www.diabetesdatarights.com/s/Virtual-Roundtable-Report-140725-HH-MQ.pdf)
worked a different list and resolved part of it: equality and non-discrimination
was deliberately made **a lens applied to every principle** rather than a
standalone one, technology-and-automation was folded into accountability and
transparency, and privacy was pulled out of choice-and-control to stand alone.
This document is written against the eight, noting the roundtable resolutions
where they sharpen a principle.

*This section reopens if:* the published Charter (2026) differs from the
candidate principles. **Re-read it on publication.** Every verdict below is
against a draft.

---

## 1. The relationship: the Charter asks, this takes

**The Charter asks for rights to be granted. This project exercises them
unilaterally.** Those are not the same lever and neither substitutes for the
other.

That gives the project exactly one thing to offer the Charter, and it is not
alignment — it is **evidence**. The January 2025 roundtables recorded that
participants were *"mindful of the potential tension between PwD desire for
choice and control over their data and security and proprietary concerns of
industry."* That tension is the defence: *what you are asking for cannot be
built safely.* A working demonstration that per-person, revocable, server-free
sharing runs on a phone does not settle the proprietary half of that objection,
but it removes the feasibility half.

**What it must not be pitched as is a substitute.** "Build your own" is not a
right. A person who cannot assemble an Android toolchain has not been given
anything by this repository, and a Charter that achieved its aims would make
most of this project unnecessary — which is the correct outcome and should be
said out loud when talking to them.

---

## 2. Access — **strong**, and stronger than the first draft of this document said

**The Charter asks:** open, secure, real-time access, in a format the person can
use. One participant's scope: *"data is all data generated, used, or relevant to
patient decisions pertaining to care."*

**An earlier draft of this section said the project "operates downstream of the
access problem — it only has data because AAPS already got it, and it prises
nothing out of the vendor's cloud." That is wrong on this hardware**, and wrong
in the direction that understates the DIY stack's achievement.

**There is no vendor anywhere in the chain.** The reference loop runs Libre 3
through AAPS's own `Libre3SourcePlugin` — the sensor is read by open-source
software on the phone. No proprietary application is installed, and the readings
do not transit the manufacturer's servers on their way to anywhere. By the time
this project sees a record, the access problem has already been solved, *and it
was solved by interoperability at the sensor boundary rather than by anyone
granting anything.*

So the honest description of the chain is:

```
sensor ──▶ open source on the phone ──▶ AAPS ──▶ diaswarm ──▶ someone you chose
```

That is PeSOS end to end, achieved without asking. **This project inherits it
and extends it one hop** — from *"I hold my data"* to *"I decide who else does"*
— which is the hop the Charter's Consent & Control principle is about.

**Two things keep this from being a clean win.**

- **It is a property of the assembled stack, not of this repository.** Someone
  on a locked pump-and-CGM combination has no such route, and nothing here
  creates one for them. The project inherits PeSOS where the DIY stack achieves
  it and cannot manufacture it where it does not.
- **Real-time is measured and Android-limited.** A follower polls every two
  minutes awake, comfortably under the CGM's five ([D17](decisions.md);
  measured at 105s and 123s). Doze stretches that arbitrarily in a pocket, which
  is why every reading is displayed with its age.

**One under-claimed contribution belongs here.** [D6](decisions.md) treats
canonicalisation as part of the protocol: version rows and retracted rows are
removed and units normalised at the emit boundary, because a consumer taking the
AAPS tables at face value over-counts insulin and carbs. That is the Charter's
*data utility and accuracy* provision — *"complete, reliable, and processed in a
consistent manner"* — implemented rather than asserted, and it is unusual.

*Verdict downgrades if:* a future BG source requires a proprietary bridge app,
at which point the chain acquires a vendor and this section is no longer true of
the reference install.

---

## 3. Consent & Control — **strong**. This is the project's thesis

**The Charter asks:** choice and control over how data are shared and with whom;
the ability to consent, and to refuse or withdraw. The January 2025 consensus
adds a specific test: control should *"provide for the ability for people to opt
out of data sharing whilst still being able to use a diabetes device with full
functionality."*

**That test is passed structurally rather than by policy.** The AAPS add-on
ships disabled, is a `DataSyncSelector` that can only drain a queue outward,
holds no pump reference and implements no constraint ([D7](decisions.md)).
Opting out is turning it off, and the loop does not notice.

Two of the reviewed charters state this principle in terms that describe the
architecture almost exactly:

- The **Electronic Frontier Alliance**: technology should *"allow users to set
  their own parameters about what to share and with whom."*
- **Indigenous Peoples' Rights in Data**, on the right to possess: *"the ability
  to exercise jurisdictional control over the ways that data flow/move/are
  queried."*

Against the incumbent the difference is categorical, not incremental: revoking
one person from a Nightscout instance means rotating `API_SECRET`, which revokes
everyone. Here it is per-person and by key, and stopping does not require a
server's cooperation ([§11](feasibility.md)).

**Three limits, all named elsewhere and all real.**

1. **Withdrawal means "nothing new will be sent", never "give it back."** §11
   forbids the word *revoke* for this reason. **This is the one that matters**,
   and nobody has established that the Charter's participants would accept it as
   satisfying their principle. Everything a reader already downloaded stays
   readable forever, and no protocol reaches that.
2. **Granularity is good, and this document is why [D4](decisions.md) now says
   so.** The sealing layer **cuts a new segment on withdrawal**, so what a reader
   keeps is bounded by *when they were revoked*, not by the epoch; and epochs are
   cut at a fixed per-subject offset — local midnight, not raw UTC — so the
   boundary does not fall in the middle of the waking day. D4 claimed otherwise
   on both counts until 2026-09-10, when writing this section surfaced the drift
   and D4 was amended. **Nothing in the system changed; the record of it did.**
3. **There is no pause.** The only controls are withdrawing a grant or turning
   the plugin off — nothing between sharing and not sharing. Under a principle
   built on *choice*, two states is a thin menu.

*Closes when:* limit 1 is put to actual people — see §12 below. **This is not a
question this repository can answer about itself**, and it is now the only one
of the three that needs asking.

---

## 4. Privacy & Data Safety — **partial**, and gated on review

**The Charter asks:** privacy by design, security proportionate to the data,
and — per the European Cancer Patient's Bill, Art 1.7 — *"the level of
confidentiality of their own data to be decided by the patient."*

**Architecturally this is the strongest claim available:** public ciphertext,
private keys, no host, no operator, and a peer that holds a stranger's history
and can read none of it. Measured in the sharpest form the design permits — a
stranger fetches byte-for-byte the same vault as the granted partner and opens
nothing ([D15](decisions.md)).

**And it fails the principle anyway, on one line: the cryptography has not been
reviewed.** X25519 + HKDF-SHA256 + ChaCha20-Poly1305 and Ed25519, composed by
hand ([D2a](decisions.md)). No privacy principle written by patients would
accept hand-composed primitives over medical data on the author's own assurance.
**[§10.6](feasibility.md)'s review gate is therefore a Charter gate, not merely
an engineering one**, and it is the single largest item standing between this
project and an honest claim under this principle.

**Four leaks that survive the architecture**, stated because the principle is
about safety and not about intent:

- **Publication is permanent** ([§7.4](feasibility.md)). Nothing here can
  unpublish. §11 forbids the sentence *"delete my data."*
- **No forward secrecy.** A leaked key opens everything it was ever wrapped for.
- **Membership and a coarse social graph** ([D19](decisions.md)). Bucket topics
  are public; joining one shows which subjects are announced there and by whom.
- **The grant log leaks count and timing** even though it names nobody
  ([D13](decisions.md)), and whether grants need to be private at all is
  [§12.0](feasibility.md) — the design's own sharpest unsolved problem.

**One near miss belongs on the record here**, because privacy-by-design is
judged by what happens when it goes wrong. Until it was caught on hardware, peer
announcements included the phone's **public IP address**, handed to anyone
holding the subject key ([D18](decisions.md), superseded). An endpoint id is a
pseudonym; an IP address is a place. It was found, filtered at both ends, and
written down rather than quietly fixed.

*Closes when:* §10.6 completes. Not before, and no amount of internal testing
substitutes.

---

## 5. Interoperability — **fail**, half of it deliberately

**The Charter asks:** industry-wide standards, data portability, and — per the
DIG_IT recommendation — *"build solutions using technologies that patients are
already using, to ensure interoperability, easier transition for the users, and
lower costs for patients."*

**This project speaks exactly one dialect and invented all of it**: a bespoke
record vocabulary (`spec/records.md`), a bespoke wire (`diaswarm/5`), a bespoke
invite scheme ([D16](decisions.md)), no FHIR, no Nightscout-shaped API, and no
export of any kind. On the principle the Charter names second, this is close to
the opposite of what is asked.

**The failure splits in two, and only one half is fixable.**

**Fixable — the data.** A granted reader already decrypts records; emitting them
in a shape other software understands is a function over an open vault. This is
now [§10.7](feasibility.md): **Nightscout JSON** (`entries`, `treatments` — the
DIG_IT recommendation applied literally) and **FHIR bundles** (`Observation`,
`MedicationAdministration`, `NutritionIntake` — what an institution can actually
receive). Computed at the reader, on the reader's device, so no exporter ever
becomes a custodian. It depends on nothing else in §10 and is unblocked today.

**Not fixable — the access.** *You cannot hand a clinician a URL.* Every reader
must run software, because access is key custody ([D1](decisions.md)) and a
transport that decided who to serve would reintroduce the server
([D14](decisions.md)). The Charter will score that as a barrier and it is one.
**It is the price of the privacy claim in §4, and it should be stated as a price
rather than left looking like an oversight.**

**iOS is a third-order interoperability failure** ([D12](decisions.md)) and
reads worse under the Charter than in engineering terms: under the equality lens
the roundtables adopted, excluding every iPhone follower is not a scope decision
but a differential access outcome.

*Reopens if:* §10.7 lands and round-trips against real consumers — a Nightscout
instance ingesting `entries`, one FHIR server validating a bundle. That moves
the data half to pass. The access half stays failed by construction.

---

## 6. Benefit — **fail**, on intent only

**The Charter asks:** that data benefit the individual and, where shared, the
wider community, without punitive or — the roundtables preferred the broader
word — *exploitative* use. Participants drew a temporal line: **contemporary
data should benefit the individual; historic data may benefit the community**,
and neither should harm the individual.

**That line is almost exactly [D5](decisions.md) and [§9.9](feasibility.md)**,
including the part the Charter cares most about and most projects omit — data
coming *back*: cohort baselines the individual cannot compute alone, and a
record of what their data supported. §9.9 states the rule for withdrawal in the
Charter's own register: *nothing new will be done with it.*

**None of it is built.** No commons gateway, no CSV, no Parquet, no export at
all — grep the repository. On this principle the project currently scores as
design intent, and design intent is what the Charter is already receiving from
industry.

Two further gaps under this principle:

- **The flagship is individual, and the Charter's framing is collective.**
  Sharing with a partner or a parent is a narrower benefit than the one the
  roundtables discussed.
- **Cohort re-identification is unanswered** ([§12.4](feasibility.md)). A
  5-minute CGM trace is close to a fingerprint. This is the first question an
  ethics committee asks and there is no answer yet.

*Closes when:* §10.5 exists with the return path in its first version, not its
third.

---

## 7. Trust & Transparency — **strong**, and the most transferable thing here

**The Charter asks:** transparency about what data are collected, how they are
stored and used; accountability; and — added at the January 2025 roundtables —
the ability to rectify incorrect data, with one participant raising whether
decisions involving data should be auditable.

**[§11](feasibility.md) is the asset.** A standing list of sentences that must
never be said about this system, each paired with the true version: *"we can
show you who read your data"*, *"delete my data"*, *"usage is audited"*,
*"revoke"*, *"trustless"* — forbidden, with the reason given as *"a lie a person
might rely on at 3 a.m."* Alongside it: AGPL, a Limits section in the README, a
decisions log carrying its own reopening criteria, and two independent
implementations checked byte-for-byte because *"two implementations that agree
are evidence and one implementation is an assertion."*

**A "what must never be said" list is a self-imposed transparency standard of
exactly the kind the Charter is asking industry to adopt**, and it is portable
— it needs no swarm, no crypto and no p2p. If one artefact from this repository
is worth putting in front of the Charter authors, it is that one.

**Two failures under this principle.**

- **Rectification is impossible by construction**, and this is sharper than the
  generic no-deletion caveat. A corrected record propagates *forward* — canon
  re-emits it and the next segment carries the correction — but the erroneous
  segment remains sealed, published, and readable by everyone ever granted that
  epoch. **The wrong number is permanent even though the right one arrives.**
- **Reads are unauditable, deliberately** ([D3](decisions.md)). What survives is
  a signed, tamper-evident record of *grants*. That is more than the incumbent
  offers and less than "usage is audited", and §11 already forbids saying
  otherwise.

---

## 8. Technology & Human Intervention — **strong**

**The Charter asks**, in the Scottish Dementia Technology Charter's phrasing
that the review carries forward: *"technology augments — but does not replace —
human intervention."* Folded into accountability and transparency at the January
2025 roundtables, extended to cover AI in data processing.

**[D7](decisions.md) is the load-bearing answer**: the add-on is structurally
incapable of dosing, and the residual risk is named honestly as *a plugin that
fails to construct takes the app with it, and an app that will not start is a
loop that has stopped.*

**The follower UI is the subtler answer.** Every reading is shown with its age,
because *"a follower's dangerous failure is not an error on screen, it is a
value that looks fresh and is nine hours old"* — a human-factors decision of
precisely the type this principle exists to require. §11 separately forbids any
claim that a follower's view is suitable for a dosing decision.

**No automated decision-making, no model, no inference.** The AI clause has
nothing to bite on, which is a pass by absence rather than by design.

---

## 9. Responsibility — **thin, and structurally so**

**The Charter asks:** that stakeholders be accountable for what they do with
diabetes data.

**Removing the custodian removes the accountable party.** That is the flip side
of *"no server, no hosting bill, no operator to trust"* in the README's trade
table, and it should appear in that table's right-hand column. There is no
operator to hold to account because there is no operator.

What exists instead: AGPL, a `SECURITY.md`, an explicit disclaimer of AAPS
affiliation, no APK distributed and none ever, and the Limits section. What does
not exist: an organisation, a funder, a support channel, or anyone to ask. §11's
own comparison table scores this honestly — Nightscout has *"thousands of
people"* and this has *"nobody."*

**One open question sits squarely under this principle and is not currently
framed as a rights question.** [§12.6](feasibility.md) — *who holds the keys
when the subject cannot?* Diabetes has minors and has emergencies. **The
Charter's roundtables explicitly included parents of children with diabetes**,
so this is not an edge case in that constituency; it is a large share of it.

---

## 10. Equality and non-discrimination — the lens, not a section

The January 2025 roundtables concluded it was *"more important to have all
principles viewed through a lens of equality and non-discrimination rather than
have a standalone equality and non-discrimination principle"*, with one
participant specifying that all principles should apply *"universally without
reservation based on race, gender identity, ethnicity, religion or country of
origin."* Participants specifically flagged the Global South and marginalised
populations.

**Applied to this project the lens is unflattering, and one part of it is not.**

- **The user base is about the most technically privileged population in
  diabetes**: a self-built AAPS install, Android, an SDK and NDK, a signing
  certificate, and a phone excused from battery optimisation.
- **iOS followers are excluded outright** ([D12](decisions.md)) — the exclusion
  lands on people who did not choose to run a DIY loop and merely want to see
  someone's glucose.
- **Storage only grows** — ~78 MB/year for a pool share, with no pruning, which
  is not neutral on a low-end device or a metered connection.
- **Against that, it removes a recurring cost.** Nightscout means a host at
  roughly $5/month plus a database, or the operational burden of self-hosting;
  this is a toggle in an app already installed. *A person with diabetes running
  a database in production so their partner can see their glucose* is an equity
  problem, and this removes it.

Both are true. Neither cancels the other, and the second does not license
claiming the first away.

---

## 11. What must never be said to the Charter, or about it

An extension of [§11](feasibility.md)'s list, for this specific audience.

- **"diaswarm implements the Diabetes Data Rights Charter."** It fails two of
  the eight principles outright and is gated on external review for a third. The
  true version is *"here is a per-principle assessment including the failures."*
- **"This is what data rights look like in practice."** It is what they look
  like for someone who can build an Android application from source. That is not
  a right, it is an aptitude.
- **"You don't need the Charter, you need better software."** Straightforwardly
  false, and insulting to seven roundtables. The access win in §2 came from
  open-source interoperability at the sensor boundary; it did not generalise to
  anyone on locked hardware, and only a rule can do that.
- **"Nightscout is the problem."** The Nightscout Foundation is listed among the
  Charter's funders. The argument to make is *this removes the server the
  community currently has to run* — which is true and is §11's own framing —
  and never a comparison that reads as an attack on the incumbent's community.
- **Anything from §11's forbidden list**, which does not stop applying because
  the audience is sympathetic. Especially *"you control your data"* without the
  permanence caveat, to an audience whose Consent & Control principle they will
  reasonably read it against.

---

## 12. Open actions

1. ~~**Fix [D4](decisions.md) before showing this to anyone.**~~ **Done
   2026-09-10.** D4 described epochs as *"one UTC day each"* and asked whether
   people accept a revoked reader keeping *"up to 24 more hours"*; both had been
   superseded on 2026-09-09 by `4a8c6e2` and the index was never updated. **This
   is the drift decisions.md exists to prevent** — the second instance, after
   D3 — and it was found only because an outside standard asked the question in a
   shape that made the contradiction visible. Kept in this list as the argument
   for doing that again.
2. **Take the withdrawal-semantics question to them** — the one that survives.
   D4's stated mechanism is *"a question for people, not for this repo"*, and
   the Charter project is 40+ people with diabetes across 16 countries, a
   co-production method, and a public contact address
   (`diabetesdatarights@contacts.bham.ac.uk`). The question is **not** granularity
   any more; it is: *is "nothing new will be sent after you stop it" an
   acceptable meaning of "withdraw consent"?* §11 requires that wording and
   nobody has checked whether it satisfies the people whose principle it is.
   §3's limit 1 closes on this and nothing else does.
3. **Ship [§10.7](feasibility.md).** Nightscout JSON and FHIR bundles at the
   reader. It is the cheapest movement available on the worst-scoring principle
   and it is blocked on nothing.
4. **Treat [§10.6](feasibility.md) as a Charter gate**, and say so when the
   review is commissioned. §4 does not close without it.
5. **Re-read the published Charter in 2026.** Every verdict here is against
   candidate principles from a draft, and §0 says what that costs.
6. **Offer §11 rather than the swarm.** The "what must never be said" list is
   the artefact that transfers to a manufacturer without requiring anyone to
   adopt anything from this repository.

---

## Sources

- [Diabetes Data Rights Charter](https://www.diabetesdatarights.com/) — mission,
  the eight candidate principles, PeSOS, partners and funders.
- [Virtual Roundtables Report, 29–30 January 2025](https://www.diabetesdatarights.com/s/Virtual-Roundtable-Report-140725-HH-MQ.pdf)
  — the fifth and sixth roundtables; where the principle set was resolved.
- [Patient-led Data Rights Exploratory Review, December 2024](https://www.diabetesdatarights.com/s/Patient-led-Data-Rights-Exploratory-Review-Final-Dec-2024.pdf)
  — 33 comparable charters, and the source of the Electronic Frontier Alliance,
  Indigenous Peoples' Rights in Data, Glioblastoma, DIG_IT and Scottish Dementia
  quotations used above.
- Reed-Berendt et al, [*Towards better Diabetes Data Rights*](https://onlinelibrary.wiley.com/doi/10.1111/dme.70160),
  Diabetic Medicine (2026).
- [*The urgent need for a diabetes data rights charter*](https://pubmed.ncbi.nlm.nih.gov/41135555/),
  The Lancet Diabetes & Endocrinology.
