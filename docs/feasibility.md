# A Swarm for Diabetes Data — Feasibility

**Status:** Assessment, 2026-09-08. No code.
**Reviews:** `whanau_voice/docs/{decentralisation-design,seeder-architecture,local-first-decentralised-tier,consent-architecture-assessment}.md`
and `consent-usage-layer/docs/design.md`
**Question:** can a person running AAPS keep writing to their own phone exactly as
now, and additionally share that data with named others, revocably and with the
use visible to them?

---

## 1. Verdict first

**Feasible, and cheaper than it looks — but one of the three promises does not
survive contact with the primary use case, and it should be conceded on page one
rather than discovered.**

- **Local-first**: already true. AAPS is the source of truth today; nothing has
  to change about how it writes.
- **Revocable**: achievable properly, by key rotation, with the honest promise
  being *prospective* — "nothing new after you say stop", never recall. This is
  a real improvement on the incumbent.
- **Auditable**: **only partially, and weakest exactly where the flagship use
  case is.** §6 is the whole of that argument and it decides the shape of
  everything else.

The volumes make this almost embarrassingly tractable (§4), the AAPS integration
seam already exists and has a working template (§5), and the hardest problems
turn out to be the ones the whanau_voice work already solved — with one
requirement inverted in sign (§3.2).

## 2. What the whanau_voice work established that carries

That body of work is further along than its "exploration" labels suggest, and
the transferable parts are the judgements, not the code.

**The load-bearing idea.** *Decentralisation comes from key custody and network
membership, not from topology* (`decentralisation-design.md` §11). Whether the
client is a peer is a library question; whether anyone who holds the bytes can
read them is the architecture. That reframe is what unstuck the design after the
earlier Holochain attempt, and it applies here unchanged.

**A holder that cannot read is a holder anyone can be.** "A seeder breach is not
a data breach — it holds noise." That single property is what makes third-party
hosting socially and legally possible, and it is what would let one person in a
diabetes swarm hold three others' data without becoming responsible for it.

**R6 — you can freeze a three-opcode protocol you wrote, and you cannot freeze
someone else's 0.x crate.** The reason a framework was rejected and 1,400 lines
of Rust were written instead. **This constraint is sharper here, not weaker.**
whanau_voice at least has partner organisations who can be asked to `docker
pull`. AAPS users run builds that are years old, update on no schedule, and
answer to nobody. A wire format that moves is a swarm that silently partitions.

**The measurement discipline.** `seeder-architecture.md` §6.1 — two months of a
decision held on an unmeasured extrapolation from a file size, settled in half an
hour with a phone, and two flattering benchmark runs thrown away because the rig
favoured the preferred answer. *"Documenting a confound is not controlling for
it."* Worth re-reading before any claim here about battery or bandwidth.

**The honesty section.** `seeder-architecture.md` §10 — "what is true", "what must
never be said", "the stretch, named". Copy the structure verbatim. §11 of this
document is that section for this project.

**The adoption ladder.** `consent-usage-layer` — *lead with usage, not consent*.
A usage record is append-only, blocks nothing, needs no change to anyone's access
control, and is the thing that does not exist today. Consent follows once the
record does, because a grant nobody can see being honoured is a promise. That
sequencing is right here too and it decides the staging in §9.

## 3. What does not carry, and one thing that inverts

### 3.1 The browser constraint — gone entirely

`local-first-decentralised-tier.md` §8 calls the browser the constraint that
shapes everything: *"None of these run in a browser tab… 'no app install' and
'true peer-to-peer' are in direct conflict."* It is the reason the earlier
Holochain work stalled and the reason the whole design collapsed to
*WASM key layer → relay → always-on peer*.

**AAPS is a native Android app.** It can hole-punch, hold long-lived
connections, run a full node, and carry a Rust library over the NDK. There is no
WASM budget, no 0.93 MB payload argument, no relay-only subset. The constraint
that dominated that design simply is not present.

That does **not** mean the frameworks it rejected come back — they were
re-rejected in `seeder-architecture.md` §3 on firmer ground (R2, R4, R6), and
those grounds mostly survive. But it does mean the rejections have to be re-run
rather than inherited, because two of them were decided on a requirement that no
longer applies and one of them **flips sign**.

### 3.2 The inversion: expiry becomes durability

This is the single biggest difference between the two problems and it needs to
be stated loudly, because almost every mechanism in `seeder-architecture.md`
exists to serve the requirement it reverses.

| | whanau_voice | Diabetes swarm |
|---|---|---|
| Principle | **P3 — nothing promises the kōrero survives** | **The value is entirely longitudinal** |
| Requirement | **R2 — replication must not extend a lifetime** | **RQ3 — data must survive for years** |
| Mechanism | `age` beside `hash` in `LIST`; `preserve_age` carries the original clock across a copy; the sweep deletes | The sweep is the thing you must *not* build |
| Why | No permanent custody to fund; the participant's download is the lasting copy | Every finding in this project came from history: the site-change effect from six events, the ISF anchor fix from 25 days, TddAdapterV2 from 7-day windows |

whanau_voice's cheapest property — *nothing outlives its window, so there is no
archive to fund or defend* — is unavailable here. Permanent custody comes back,
and with it the durability problem that decision 4 deleted.

**The compensation is that the data is tiny (§4).** whanau_voice could not fund
permanent custody of 44 MB per half-hour of audio. A decade of one person's
diabetes is smaller than a single kōrero.

**Consequence for reuse:** `crates/seed` is directly reusable, but S1
(`preserve_age`) and the retention sweep are the two pieces to *remove*, and
Hypercore's rejection on R2 does not carry — see §8.2.

### 3.3 Other things that do not transfer

- **Identity is easy here.** whanau_voice spends §4 and all of
  `consent-architecture-assessment.md` §6 avoiding accounts for a kaumātua with a
  cheap phone and no login. AAPS already has a device, a keypair is free, and the
  counterparties are named people the subject already knows. The per-relationship
  handle problem (`consent-usage-layer` §6.2) — the politically hardest thing in
  that design — does not arise.
- **The governance gate does not exist.** There is no `/export` decision, no
  board, no funder. Nobody has to say yes. That removes the biggest programme
  risk and removes the funding, which is §10's problem.
- **Karo-as-processor has no analogue.** whanau_voice's grants exist so a server
  can decrypt, compute and discard. Nothing in a diabetes swarm needs to run
  server-side inference. That deletes an entire class of trusted-operator
  concession — and creates a worse one (§6).

## 4. The volumes, measured

From the largest real snapshot in `realdata/` — 44 days of live loop, this
project's own data. That directory is **outside this repo**, in the working
directory this project was split out of; `.gitignore` refuses `*.db` and real
snapshots never enter version control. `tools/mkfixture.py` builds a synthetic
database with the same hazards, so everything below can be re-run by someone who
has no diabetes data.

| | rows | on disk |
|---|---|---|
| `deviceStatus` | 13,468 | **20.8 MB** |
| `apsResults` | 11,124 | **9.9 MB** |
| `glucoseValues` | 23,948 | 2.6 MB |
| `temporaryBasals` | 18,321 | 2.2 MB |
| `userEntry` | 7,795 | 0.9 MB |
| `boluses` · `carbs` · `therapyEvents` | 1,458 · 345 · 146 | < 0.3 MB |
| **total file** | | **45 MB / 44 days** |

**Two-thirds of it is JSON blobs nobody would share.** `deviceStatus` and
`apsResults` are loop telemetry — Nightscout plumbing and algorithm debug — and
carrying them would triple the payload for no clinical content. Excluding them:

- **~6 MB per 44 days ≈ 50 MB/year raw**, and this is a maximally regular time
  series — fixed 5-minute grid, small integer-ish values, long runs of unchanged
  fields. Delta-and-varint encoded it is comfortably **under 5 MB/year**, likely
  1–2.
- **A decade of one person fits in about the space of one 30-minute kōrero.**
- A cohort of 1,000 people over 10 years is single-digit GB. A seeder for that
  is a Raspberry Pi with a USB stick.
- Write rate is **~350 records/day** — one small append every four minutes.
  Nothing in this document is stressed by that.

**Hygiene must be fixed at the emit boundary, not downstream — and the cause is
not what this document first said.**

`glucoseValues` holds 23,948 rows, which looks like ~544 readings a day against
the 288 a 5-minute sensor can produce. That was attributed here to an xDrip
double-broadcast at ~1.85×. **Measured, it is not: it is AAPS version history.**

| snapshot | rows | current (`referenceId IS NULL`) | distinct timestamps |
|---|---|---|---|
| snap_chk | 23,948 | 11,974 | 11,974 |
| f | 31,532 | 16,400 | 16,400 |
| snap_0721 | 10,498 | 5,435 | 5,435 |
| androidaps2 | 1,708 | 854 | 854 |

The current rows account for **every distinct timestamp — and every distinct
5-minute bucket, ratio 1.000** — on all four snapshots.
A version row is written when NSClient stamps a `nightscoutId`; the pair differs
only in `version`, `referenceId` and that id. Filter `referenceId IS NULL` and
there are **zero duplicate CGM timestamps**, at **272.4/day — 95% of the 288**,
which is ordinary sensor uptime.

**Two spans, and they are not interchangeable.** This section says 44 days and
the tool says 48.5: the snapshot's *records* span 48.5 days, its *CGM* spans
44.0, because the first CGM reading arrives 4.6 days after the first record of
any other kind. The rate above was briefly reported as 246.6/day by dividing
readings by the longer span — counting days on which no CGM existed. Row counts
here use the 48.5-day file; rates use the span of the thing being rated.

The remedy is unchanged and the reason for it is stronger: a swarm that ships the
tables as-is exports roughly **2× the real insulin and carbs** to every consumer,
and every model fitted on it is wrong in the same direction.
**Canonicalisation is part of the protocol, not a consumer's problem** — and the
5-minute debounce stays as defence in depth, since a genuinely double-broadcasting
source would otherwise reach consumers unnoticed.

**Confirmed end-to-end.** `tools/canon.py` produces **1.66 MB of NDJSON over 48.5
days, 0.19 MB gzipped — 1.5 MB/year projected** (19,128 records; 1,655,863 and
194,518 bytes exactly), which lands inside the 1–2 MB estimate this section rests
on. `tools/test_canon.py` pins the filters those numbers depend on.

## 5. The AAPS integration seam

This is the part that is genuinely already built, and it is better than expected.

`core/interfaces/.../sync/DataSyncSelector.kt` is a per-plugin interface: typed
`DataPair` wrappers over all fourteen record types, each carrying an `id`, plus
`queueSize()`, `doUpload()` and `resetToNextFullSync()`. Each sync plugin keeps
its **own** high-water marks, so adding a third consumer disturbs neither
NSClientV3 nor xDrip.

`plugins/sync/xdrip/` is a complete worked example at **~1,100 lines** across
seven files — plugin, selector, a `Worker`, its own key namespace, its own
events. A swarm plugin is a fourth sibling next to `nsclient`, `nsclientV3`,
`nsShared` and `xdrip`.

Three consequences:

- **The Android-side work is bounded and well-understood.** Call it the xDrip
  plugin plus a transport — not a fork, not a patch to the loop.
- **It is structurally read-only out of AAPS**, which is the isolation RQ10 needs.
  A selector drains a queue outward. Nothing in this shape can write back into
  the loop, and it must stay that way: anything that can influence dosing is
  inside the medical device's blast radius and inherits its whole risk posture.
- **AGPL-3.0.** The plugin is AGPL and source-available. That forecloses a
  proprietary swarm built on this seam, which is a feature.

## 6. The requirement that does not survive: audit

Stated plainly, because everything else in this document is easier than it and
because the temptation to soften it will be constant.

**Once a named recipient holds the key and the bytes, nothing makes them record
that they read.** Every mechanism in the reviewed designs — whanau_voice's signed
access attestations, `consent-usage-layer`'s hash-chained ledger, Certificate
Transparency's gossip — produces *an account the reader cannot quietly rewrite*.
None produces *an account the reader cannot simply omit*. That distinction is the
whole of it, and `local-first-decentralised-tier.md` §3.3(d) already concedes it
in the audio case: *"This is not cryptographically enforceable — nothing compels
Karo to write it."*

The audio case gets away with it because **Karo is a processor**: it fetches
under grant, computes, discards. Fetching is observable, so the fetch *is* the
audit event and it is nearly complete.

**A diabetes follower is not a processor.** The flagship use case — a parent
seeing their child's CGM at 3 a.m., a partner getting a low alarm — requires the
recipient to hold a **live local replica**, because the whole point is that it
works when the phone is asleep, the network is bad, and nobody is fetching
anything.

**So there is a real trade, and it cannot be designed away:**

| | Replicate to the recipient | Fetch on demand |
|---|---|---|
| Works offline / alarms at 3 a.m. | **Yes** | No |
| Reads are observable to the subject | **No** | Yes, at seeder granularity |
| Revocation stops new data | Yes | Yes |
| Revocation stops re-reading old data | No | No |

**The strongest true claim is therefore:** *you can see everyone you have granted,
every grant and withdrawal with who made it and when, and every time data left
your phone toward whom. You cannot see them open it.*

That is materially less than "usage can be audited". It is also materially more
than anything available today (§8.1). Say it in those words. The alternative — a
usage timeline that looks complete and is not — is worse than no timeline,
because a person will make a safety judgement on it.

**Where the full claim survives:** research and analytics access, which is
processor-shaped exactly as Karo is. A cohort query that fetches under grant,
computes and discards is auditable in the strong sense.

**And this is a choice of architecture, not a fact about the world.** §8.2 sets
out a model — Holochain private entries plus assigned capability grants — where
*every* recipient is a caller rather than a replica-holder, and every read is
therefore a remote call the subject's own node serves and can log. That restores
the strong audit claim across the board. What it costs is the 3 a.m. alarm,
because a phone in a drawer cannot answer a call.

So the honest statement of §6 is not *"audit is impossible"*. It is:

> **Audit and offline availability are in direct conflict, and no design gets
> both.** Whichever is chosen, the other is what you are giving up.

## 7. The shape of the answer: public ciphertext, private keys

**Corrected 2026-09-08.** An earlier draft proposed Holochain *private* entries
as the way to hold data. That is wrong, and the objection is decisive:

> A private entry lives only on its author's source chain, so every read is a
> call the author's phone must answer. **That makes the subject a personal
> server.** It is the opposite of a swarm — availability collapses to one
> device's uptime, which is the single thing a distributed store exists to fix.

The point of a swarm is that data is **highly available to anyone the subject
gave access to**, whether or not the subject's phone is on. That requires the
records to be **published and replicated**, which in turn requires the records to
be **encrypted**, which in turn makes access control a matter of **who holds a
key** rather than who a storage layer decides to serve.

This is the same conclusion `seeder-architecture.md` reached — *"a seeder breach
is not a data breach; the hash is useless without a key"* — arriving from the
other end. **The swarm is the seeder set, and it is made of the users.**

### 7.1 Three layers, and only the middle one is unsolved

| Layer | What it is | Who provides it |
|---|---|---|
| **Storage** | Public, replicated, content-addressed **ciphertext**. Every member holds some of everyone's | A DHT or gossip network. Several frameworks do this well (§8) |
| **Keys** | Which named readers can decrypt which records | **The hard part.** Frameworks differ sharply here |
| **Grants** | Signed, public statements: *"S granted R purpose P over epochs E1–En until T"* | Ordinary signed records on the same store |

Layer 1 is a solved problem several times over. Layer 3 is a data model. **Layer 2
is the design, and it is where a framework is worth choosing or rejecting.**

### 7.2 Epoch keys, and what they buy

The construction that makes layer 2 tractable, and it is not novel — it is
`sealing.ts`'s content-key-per-recording generalised to a stream:

- Data is written in **epochs** — a day is a good unit. Each epoch has one
  content key. Records in that epoch are sealed under it and published.
- For each **live grant**, the epoch key is **wrapped to that reader's public
  key** and published as a tiny record beside the data.
- **Granting** = start wrapping for them. **Revoking** = stop. The next epoch's
  key is never wrapped to them and they get nothing further.

Four things fall out of that, and they are the whole reason to do it this way:

1. **Time-scoped access for free.** A researcher gets 2025's epoch keys and
   nothing else. A clinician gets the last 90 and a fresh one each day while the
   grant lives. Scope is which keys you wrapped, not a policy anyone enforces.
2. **Revocation granularity is the epoch length.** Revoke mid-day and the reader
   keeps that day. Daily epochs bound the leak to 24 hours — an honest,
   statable number rather than a hand-wave.
3. **The cost is nothing.** Five readers, daily epochs, ~100 bytes a wrap:
   **about 180 KB a year of key records.** Against 1–2 MB/year of data, key
   management is a rounding error, and that is what makes the simple mechanism
   competitive with the clever ones.
4. **The subject can be offline.** Wraps for the current epoch are published when
   the data is. Nothing further is required of the phone for a reader to catch up.

**Purpose scoping** is the same trick a second time — a separate key per purpose,
so `follow`, `clinician` and `cohort` are different key trees over the same
records, exactly as `wrappedForPurpose` does in whanau_voice.

### 7.3 The candidate mechanisms for layer 2

| | How access is granted | Grant while offline | Maturity | Fit |
|---|---|---|---|---|
| **Per-recipient wrapping** | Wrap each epoch key to each reader's public key | Only for epochs already wrapped; new grants need the subject once | Boring, obvious, auditable | **Strong.** At five readers the O(n) cost is meaningless |
| **CGKA — group key** (`p2panda-encryption` *data mode*, MLS) | One group secret; rotates on member removal; joiners are given prior secrets | Yes, once the group op is published | p2panda 0.7.x; MLS is an RFC | **Strong**, and purpose-built — see §8.4 |
| **Proxy re-encryption** (Umbral / TACo lineage) | Publish a re-encryption key; proxies transform ciphertext for the reader **without the subject re-encrypting anything** | **Yes, indefinitely** — grant future access without touching the data | Spec + implementations; TACo is being forked and relaunched H2 2026 | **Interesting**, and the only one that grants without the subject at all. Immature and needs semi-trusted proxies |
| **Attribute-based encryption** | Encrypt under a policy; issue keys per attribute set | Yes | Research-grade; usually wants an attribute authority | Elegant for purposes, wrong maturity |

**Recommendation: per-recipient wrapping is the floor and it is sufficient.** Do
not reach for a CGKA to solve a five-reader problem — but note that the CGKA
route arrives free if p2panda is taken (§8.4), because it is what that library
already does.

**Proxy re-encryption is the one worth watching**, because "grant access to
someone you have never met, over data you published last year, without your phone
being involved" is a capability nothing else here has, and it is exactly what a
research commons enrolling participants at scale would want.

### 7.4 What this architecture costs, named

Publishing ciphertext to a swarm buys availability and charges for it. All four
of these are permanent properties of the choice, not defects to be fixed later.

- **Reads become unobservable, for good.** Anyone can pull ciphertext from the
  store; there is nobody to ask and nothing to log. §6's trade is now settled for
  the personal case: **the usage record is grants and egress, never reads.** The
  commons gateway (§9.9) remains the exception, and socially rather than
  cryptographically — because it is an identifiable organisation under an
  agreement, not because the maths compels it.
- **Publication is permanent.** A DHT cannot unpublish. "Delete my data" is not
  available, and the design must never imply it is. What *is* available:
  nothing new is added, and no new key is ever issued.
- **A leaked epoch key is forever.** Bounded to its epoch, which is the argument
  for short ones — but a reader's compromised device exposes every epoch they
  were ever given, permanently. **This is strictly worse than the fetch model**
  and is the sharpest thing to say out loud.
- **The grant records leak the social graph.** A signed public record saying *"S
  granted R"* tells the swarm who your clinician is, when you enrolled in a study,
  and when you stopped. That is a real disclosure and it is easy to miss because
  the *data* is encrypted. Grant records need blinding or their own encryption —
  see §12.

## 8. The frameworks, against these requirements

Requirements first, in the house style, so the table below is judged against
something rather than against taste.

| | |
|---|---|
| **RQ1** | The local write path is never blocked, delayed or made to depend on a network. AAPS is a loop; a sharing layer that can sit in front of a pump command or a CGM read is disqualified, not debated |
| **RQ2** | The phone is the source of truth and works offline indefinitely |
| **RQ3** | Data survives for years. **Inverts whanau_voice R2** (§3.2) |
| **RQ4** | Named recipients, each with their own scope — a partner, a clinician, a cohort |
| **RQ5** | Revocation is prospective and enforced by key, not by policy: after T, nothing new |
| **RQ6** | Grants, withdrawals and egress are visible to the subject. Reads, only where the recipient must fetch (§6) |
| **RQ7** | No operator and no server anyone must fund forever |
| **RQ8** | The wire protocol survives a **release season**, not forever. See the correction below — this started as whanau_voice R6 and does not survive contact with AAPS |
| **RQ9** | Battery and data are safety properties on a loop phone, not preferences |
| **RQ10** | Hard isolation from dosing. Nothing here may write back into the loop |

> **On the two numbering schemes.** These `RQ` tokens are the *requirements* this
> document judges frameworks against. `docs/decisions.md` numbers the *decisions*
> `D1`–`D9`. They were both `D` until a review pointed out that `D3` meant
> "data survives for years" here and "audit is given up" there, in two documents
> that cite each other constantly.

### 8.0 A correction to RQ8, before anything is judged against it

**RQ8 was inherited, and it is wrong as originally written.** It came straight
from whanau_voice R6 — *"a partner's box running whatever they pulled eighteen
months ago must still interoperate"* — and that premise is false here.

`plugins/constraints/.../versionChecker/VersionCheckerPlugin.kt` applies an
expiry as a **loop constraint**:

```kotlin
return if (endDate != 0L && dateUtil.now() > endDate)
    maxIob.set(0.0, rh.gs(R.string.application_expired), this)
```

An expired AAPS build has **maxIOB forced to zero — it stops dosing.** So there
is no population of active loopers on eighteen-month-old builds; there cannot be.
Every phone still running the loop is on a release from roughly the last year,
and AAPS enforces that harder than any app store does.

**RQ8 therefore relaxes from "freeze the wire forever" to "stay compatible across
about a year of releases".** That is a requirement a 0.x framework can meet —
especially for a contributor who tracks its release cadence and can see breaks
coming. It removes the objection that did most of the work in §8.4 against
p2panda, and part of the work in §8.2 against Holochain.

What survives of RQ8: **followers are not loopers.** A follower app on a relative's
phone has no expiry mechanism and no reason to update. Compatibility has to be
carried on that side, or the follower app needs its own nag.

### 8.1 The incumbent, which is the thing to beat

**Nightscout.** One self-hosted instance per person; per-subject tokens with
roles (`readable`, `denied`); revocation by rotating `API_SECRET`, **which
invalidates every token at once**; no access log for the subject. Followers cache
locally regardless, so even the nuclear revoke does not reach what they already
have.

So the incumbent is: **coarse revocation, no audit, and a server every person
must fund and maintain.** It is also the thing thousands of people actually use,
which is the bar that matters. Any swarm is competing with a working system on
convenience and losing, and must win on the two axes where Nightscout is weakest
— which happen to be precisely RQ5 and RQ6.

The research path is worse still: donation to the OpenAPS Data Commons via Open
Humans is a one-way, irrevocable upload. There is no withdrawal and no use
record. **That is the widest gap in the landscape and the narrowest technical
problem.**

### 8.2 Holochain

Twice revised, and the second correction matters more than the first.

**Draft 1** rejected it because a DHT scatters data across peers the subject did
not choose. **Draft 2** answered that with private entries and assigned
capability grants. **Both were wrong**, and §7 says why: private entries make the
subject a personal server, and scattering *encrypted* records across peers is not
a leak — it is the entire point.

So the right question is: **does Holochain provide layer 2?**

**It does not.** Capability grants gate **zome calls**, not DHT reads. A public
entry is published to the DHT and held by whichever agents' arcs cover it; any
agent in the network can retrieve it. There is no read-authorisation layer over
public data, and there is not meant to be — Holochain's security model is
validation and provenance, not confidentiality.

| Layer | Holochain |
|---|---|
| **Storage** | **Excellent.** Agent-centric, validating, self-replicating across users. Exactly the swarm |
| **Keys** | **Nothing.** Bring your own encryption and key distribution |
| **Grants** | Source-chain entries work fine as signed grant records |

That is not a rejection — it is a scoping. Holochain would be **the storage and
validation layer**, with §7's epoch-key construction built on top as application
code. The question then becomes whether the swarm is worth what it costs:

- **RQ9/RQ10 — the conductor.** A process, not a library; Volla runs one per phone
  as a system service. On a loop phone that is a second always-on runtime
  competing with the thing keeping someone alive. Contributable (§10.3), not free.
- **RQ8 — breaking releases.** 0.7.0's database and wire protocol are incompatible
  with 0.6. Survivable on the AAPS side per §8.0; real on the follower side.
- **You would be paying for the conductor to get replication**, having written
  the hard part yourself.

**Verdict: a credible storage layer whose key layer you supply.** Weigh it
against §8.4, which supplies both.

### 8.3 Pear / Holepunch

The most production-proven family here, and the pieces line up unusually well:
**hypercore** is a signed append-only log — the exact shape of AAPS data;
**autobase** multiplexes several writers' logs; **autopass** is a shipped
application built on both (PearPass); **blind pairing** does invite-based device
and peer linking *without publishing the key on the network*; **blind seeding**
gives durability without custody, which is `seeder-architecture.md`'s central
property available off the shelf; and **Bare** is a genuinely small JS runtime
that embeds on Android and iOS rather than assuming a desktop.

**The R2 rejection does not carry.** `seeder-architecture.md` §3 rejected
Hypercore first for being append-only against a data model that must forget.
Under RQ3 that reverses completely: append-only is the *correct* grain for a
diabetes time series, and it is the one framework whose data model needs no
argument.

Two objections survive, and one is decisive:

- **RQ5, decisive.** *Knowing the key grants irrevocable read.* A hypercore read
  capability cannot be withdrawn — revocation means writing a **new core** and
  re-sharing it with everyone who remains, and any follower who kept the old key
  keeps reading it forever. That is the exact problem Meadowcap and CGKA exist to
  fix, and it is the single requirement the user named first.
- **RQ4/RQ9, survivable.** Hyperswarm discovers over a public DHT and connects by
  UDP hole-punching with no traffic-carrying fallback. On phones this is mostly
  fine — the corporate-firewall case that killed it for whanau_voice (R4) barely
  applies — but carrier-grade NAT is real, and a public DHT announcement leaks
  the existence and liveness of a topic.

**Verdict: the best availability layer, the wrong access-control layer.** Same
conclusion whanau_voice reached, reached for a different reason, which is the
kind of agreement worth trusting.

### 8.4 p2panda — the closest match to §7, deliberately

`p2panda-encryption` ships **two modes, and the distinction is exactly §7's**:

- **Message encryption** — Double Ratchet, per-message keys, strong forward
  secrecy. For group chat. **Wrong for this**: a reader who joins late, or who
  was offline, cannot read what they missed.
- **Data encryption** — a shared symmetric group key (XChaCha20-Poly1305), keys
  **rotated on member removal**, and **joiners are given the list of prior
  secrets so they can decrypt existing data.** Key distribution is HPKE-based and
  O(n) rather than pairwise.

**That is the requirement, described in the library's own terms.** Long-lived
encrypted data on a replicated store; membership decides who can decrypt; removal
rotates forward; a new reader is given prior secrets.

⚠️ **The rest of that paragraph used to read "and because it is a *choice* per
join, the design can withhold history from a researcher while granting it to a
clinician." That was wrong, and it has been measured** —
`spike/p2panda-seal/FINDINGS.md`. In 0.7.1 `add()` takes no subset argument and
welcomes a joiner with the **whole** secret bundle: a member added at the end of
the spike opened all ten epochs, including six from before it existed. The
obvious workaround — trim the bundle, add, restore — produced a member with
*zero* secrets rather than a windowed one.

**Time-scoping therefore comes from group partitioning**, not from a per-join
choice: a cohort with a 90-day window needs its own group, whose bundle only ever
holds that window. That is §7.2's "purpose scoping is the same trick twice"
applied to time as well — more groups to manage than this document assumed, and
the one place where §7.3's per-recipient wrapping is strictly more expressive,
since there a window is simply which keys you wrapped.

`p2panda-auth` supplies layer 3 — a decentralised authorisation CRDT with
per-member permissions, which is the signed grant record. `p2panda-spaces`
integrates the two. All of it runs over raw bytes, so it can sit on any storage
layer, and it is built on iroh 1.0.

| Layer | p2panda |
|---|---|
| **Storage** | Good — `p2panda-net`, or bring your own; the crypto crates work over raw bytes |
| **Keys** | **`p2panda-encryption`, data mode. Purpose-built for exactly this** |
| **Grants** | `p2panda-auth` — signed, eventually-consistent group membership |

**The gap has narrowed, and the earlier account of it was wrong. Checked
2026-09-08:**

| | |
|---|---|
| Latest release | **v0.7.1, published 21 August 2026** — eighteen days ago |
| Cadence | 0.5.0 Jan · 0.6.0 May · 0.7.0 Jul · 0.7.1 Aug, all 2026 |
| `p2panda-spaces` | **Published on crates.io at 0.7.1**, and listed on p2panda.org as an available crate |
| Tracking issue #774 | **Closed**, merged |
| Still open | Documentation; key-bundle automatic rotation and renewal; concurrent encryption messages; group promotion and demotion |

**Two corrections.** This document previously dated v0.7.1 to **August 2025** —
off by a year — and described `p2panda-spaces` as *"unreleased and on a branch"*,
which was the stated blocker on the recommended track. A review then compounded
the first error by inferring a stalled cadence from the wrong date. Both are
wrong in the same direction: **the framework choice is firmer than this document
claimed, not shakier.**

There is still no 1.0, and the remaining open items are real — key-bundle
rotation and concurrent encryption messages are membership-change machinery, and
revocation is entirely about membership change. But they are open items on a
released crate, not an unreleased branch. Under §8.0 the 0.x objection is
survivable.

**Verdict: the recommended track.** It is the only candidate that supplies all
three layers, it was designed against this exact problem, and the missing piece
is the one worth contributing (§10.3).

### 8.5 Willow / Meadowcap / Earthstar

The best **data model** of anything reviewed: a communal namespace with one
subspace per person maps directly onto a cohort of individually-authored
timelines; entries are `(namespace, subspace, path, timestamp)` so a CGM series
is a natural path prefix; **prefix pruning is genuine deletion**, the capability
IPFS lacks; and the sync protocol is range-based set reconciliation, which is
exactly right for two parties who mostly already agree about a long time series.

Maturity has moved: **Meadowcap is Final**, `willow_rs` exposes the `willow_25`
parameter set, and WGPS and sideloading are in progress. Sideloading is the
"deliver by USB stick" path, which matters more here than it sounds — it is a
credible answer for a clinic visit with no network.

**Its known weakness is an advantage here.** Meadowcap capabilities are
delegable and clawed back by **expiry rather than a revocation list** — a real
objection in whanau_voice, where a grant might need withdrawing the moment
someone changes their mind. But P8 (*"permission expires; silence means stop, not
carry on"*) already turns short-lived, renewed grants into the intended
mechanism, and for a follower relationship an expiring capability that must be
renewed is exactly the right default. It still needs a key layer underneath.

**Verdict: steal the data model and the reconciliation algorithm. Its
capability-expiry model is a design to copy, not a limitation to work around.**

### 8.6 Keyhive / Beelay

Still **pre-alpha, unaudited, no published paper**, and built for Automerge
documents — many small ops with meaningful causal history, which a 5-minute
glucose grid is not. Its forward secrecy runs the wrong way for consent (a new
member can read history), which
`local-first-decentralised-tier.md` §3.3(b) correctly identifies as the single
most important extension to get right. Notably, that property is *desirable* for
a clinician joining a care team and *unacceptable* for a research cohort, so a
swarm needs both behaviours and gets to choose per grant.

**Verdict: watch. Do not build on it.** The `consent-architecture-assessment.md`
§4 analysis is the one to remember — CGKA converts *"they promise to delete the
key for future data"* into *"they never receive the key for future data"*, and
nothing more. That is worth having and is not what "self-enforcing revocation"
sounds like to a non-specialist.

### 8.7 MLS (RFC 9420) — not considered in the reviewed docs, and it should be

The gap in the whanau_voice survey. MLS is the **only standardised, audited,
IETF-track CGKA**, with a permissively-licensed production implementation
(OpenMLS) and years of adversarial review behind it. `Remove` plus a key rotation
is precisely RQ5, done properly, with the security argument already written by
people who do this for a living — which directly answers the risk
`local-first-decentralised-tier.md` §3.5 flags: *"we used an unaudited research
CGKA" is a hard conversation.*

Its standard objection is that it assumes a **Delivery Service** providing a
consistent order of commits — apparently a centralisation point, and the reason
it is usually skipped in local-first work.

**That objection mostly dissolves in this topology.** In a diabetes swarm there
is exactly one writer per person's data: their phone. **The subject's own device
can be the Delivery Service for its own group.** Membership changes are ordered
by the only party entitled to make them, and no other peer needs to agree about
anything. MLS's hardest deployment requirement is satisfied by accident.

Caveats worth checking before relying on it: the phone is offline often, so
commits must queue and apply late without breaking the epoch chain; and MLS is
designed around a live group of message-sending members, whereas most of these
"members" are passive readers who may never publish anything. Neither looks
fatal. **This is the highest-value thing to spike.**

### 8.8 iroh

Not a competitor to the above — the transport under most of them, and the only
dependency here that passes RQ8. **1.0 shipped June 2026**, MIT/Apache,
dial-by-public-key, hole-punching with relay fallback, WASM-capable, Android via
the NDK. Holochain moved onto it; p2panda is built on it; whanau_voice deployed
it (`relay.karo.wang`, upstream `iroh-relay` unmodified) and measured it on a
Pixel 7.

**Dial-by-public-key is the property that matters for RQ7.** A peer needs no
domain, no certificate and no inbound port, which is the whole reason a friend
can seed for you without running infrastructure.

### 8.9 Summary

Judged on §7's three layers rather than on maturity.

| | Storage swarm | **Key layer** | Grants | Verdict |
|---|---|---|---|---|
| **p2panda** | ✅ | ✅ **data-encryption mode, purpose-built** | ✅ `p2panda-auth` | **Recommended track** |
| Holochain | ✅ excellent | ❌ none for public entries | ✅ source chain | Storage layer, BYO keys |
| Pear / Hypercore | ✅ + blind seeding | ⚠️ key = irrevocable read | ❌ | Availability layer only |
| Willow / Meadowcap | ✅ + real deletion | ❌ caps gate sync, not rest | ✅ Meadowcap | Take the data model |
| Keyhive | ⚠️ | ✅ in principle | ✅ | Pre-alpha. Watch only |
| MLS / OpenMLS | — | ✅ audited CGKA | ⚠️ needs a DS | Fallback if p2panda stalls |
| Proxy re-encryption | — | ✅ **grant while offline** | — | Watch. Uniquely capable, immature |
| iroh 1.0 | ✅ transport | — | — | **Use it** — underneath most of the above |

**The answer is now a single recommendation rather than a trade.** Drafts 1 and 2
framed this as Holochain-for-audit versus p2panda-for-availability. §7 dissolves
that: once records are public ciphertext, **reads are unobservable either way**,
so audit is not a differentiator between frameworks — it is a property the
architecture gives up (§7.4) in exchange for the availability that makes it a
swarm at all.

What is left to compare is who supplies the key layer, and **p2panda supplies it
and the others do not.**

## 9. The target topology: phones, plus named institutional peers

**Direction, taken as given:** no seeders and no operator for the personal case —
data held by the people using the app, on ordinary phones. **But not phones only:
a research commons needs a way in and out, and that endpoint is by nature an
always-on institutional node.**

That is not a compromise on the goal. It is the correct shape, and it resolves a
tension the first draft left open — §6's audit trade appeared to force one answer
for the whole network, when in fact the network has two kinds of peer that
rightly take opposite sides of it.

### 9.0 Two classes of peer, not one

| | **Phones** | **Institutional peers** |
|---|---|---|
| Who | The subject, family, a partner, a clinician in consultation | A research commons, a cohort study, a registry |
| Online | Intermittently, in a pocket | Always, by definition |
| Holds | What they were granted, as a replica | Only what a live grant covers, fetched per run |
| Number | Small, personal, changes often | Very few, named, stable |
| §6 trade | **Replicate** — availability over audit | **Fetch** — audit over availability |
| Audit claim | Egress only: who you granted, when data left | **Full.** A processor that fetches under grant is auditable in the strong sense |

**This is the processor / system-of-record fork from `consent-usage-layer` §4,
arriving from the other direction.** A commons is a processor: it receives,
computes and discards. That is exactly the party a capability has teeth over, and
exactly the party whose reads can be counted — because it has to ask.

So the network is a **swarm of phones with a small number of named, always-on
peers at the edge of it**, and the design should stop trying to make one mechanism
serve both. §9.9 is what the institutional peer actually has to be.

### 9.1 One correction to the premise

**A Holochain network is not server-free.** It needs a **bootstrap service** for
initial peer discovery and a **signal/relay** for NAT traversal; 0.7 folded the
second into the first, which is a consolidation, not a removal. Every
NAT-traversing peer system has this — iroh has relays, hyperswarm has DHT
bootstrap nodes, libp2p has rendezvous points. Two phones on mobile data cannot
find each other, or reach each other through carrier NAT, without something in
the middle that is on.

So the contrast is not *Holochain needs nothing / iroh needs infrastructure*.
Both need the same class of stateless helper, and the honest claim is narrower
and better:

> **No seeders, no accounts, no operator, and nobody holding your data.**
> Not "no servers".

That distinction survives a hostile reading. "No servers" does not, and it is the
kind of claim that gets a project dismissed on the first question.

### 9.2 The DHT is the right half, and it needs a key layer

An earlier draft argued a DHT was the wrong tool because it places data among
peers the subject did not choose, while the requirement names its recipients.
**That conflates two different things** and §7 separates them:

- **Who *holds* the bytes** should be as many peers as possible. That is
  availability, and it is the swarm's job. Holders learn nothing — they hold
  ciphertext.
- **Who can *read* the bytes** is the named set, and it is decided entirely by
  key distribution.

So "named recipients" is a statement about **keys**, not about **storage**, and a
DHT does not contradict it. What a DHT genuinely costs is in §7.4 — permanence,
and metadata — and those are the costs to weigh, not the identity of the holders.

### 9.3 The numbers remove the need for one

This is the substantive finding, and it comes straight out of §4.

**At 1–2 MB/year compressed, every named recipient can hold the subject's entire
history.** A decade is tens of megabytes — less than one podcast episode. So the
question *"where does the data live so that it survives"* has an answer that
needs no DHT, no seeder and no third party:

> **On the phones of the people you named.** Three recipients is three complete,
> chosen, encrypted replicas. Nobody else holds anything.

That is more redundancy than a thin DHT arc gives, it is redundancy the subject
selected, and it is the only arrangement in this document where **no party who
was not granted access ever holds the bytes at all.**

**It is also strictly more decentralised than the Holochain model for this use
case**, on the axis that matters to the kaupapa — not "how many nodes" but "who
holds my data, and did I choose them".

### 9.4 What cannot be avoided, stated once

**NAT traversal.** Two phones on cellular are typically both behind carrier-grade
NAT; hole-punching fails often enough that a fallback is mandatory. That fallback
is a **relay**: it forwards ciphertext between two peers and **stores nothing**.

A relay is not a seeder, and the difference is the whole argument:

- It holds no data at rest, so there is nothing to breach, retain or subpoena.
- It is stateless and swappable — several public ones exist (n0 runs iroh relays;
  a peer can point at any of them, or at one a user runs), so no single operator
  is load-bearing.
- It costs nothing to nobody in particular, which is the property that killed
  every previous "everyone hosts their own" scheme.

**Zero storage infrastructure and zero trusted infrastructure are achievable.
Zero infrastructure is not — for this or for anything else.**

### 9.5 The Android constraint, and it is asymmetric

This is the concrete engineering obstacle, and it lands unevenly in a way that
happens to favour the design.

**Subject side — essentially free.** AAPS already runs a foreground service,
already requires a battery-optimisation exemption, and is sideloaded rather than
distributed through Play, so the policy restrictions that would block a normal
app do not apply. **A swarm plugin inside AAPS inherits all of that.** The
subject's phone can be a genuine always-reachable peer at close to no marginal
cost, because the expensive permission was already paid for by the loop.

**Follower side — this is where it bites.** A separate follower app needs its own
persistent connection, and on **Android 15+ a `dataSync` foreground service is
capped at six hours in any twenty-four**, after which `onTimeout()` fires and a
restart throws `ForegroundServiceStartNotAllowedException`. A follower app that
naively declares `dataSync` goes dark for eighteen hours a day. The ways out —
a foreground-service type that is not capped, a user-initiated data transfer job,
or asking the follower to grant their own exemption — all exist and all need
choosing deliberately rather than discovered in the field.

**iOS followers are the real gap.** No background sockets, no persistent
connection, no way to be a peer. A parent with an iPhone watching a child on AAPS
cannot participate on these terms — and that is a common configuration, not an
edge case. It is the one place a third party is genuinely required, and it should
be conceded rather than engineered around.

| | Can be a peer | How |
|---|---|---|
| Subject on AAPS | **Yes** | Inherits the loop's foreground service and battery exemption |
| Android follower app | **Yes, with care** | Must dodge the Android 15 `dataSync` six-hour cap |
| iOS follower | **No** | No background sockets. Needs push, or a relay that queues |
| Researcher / cohort | **Yes** | Always-on by nature; pulls under grant |

### 9.6 The precedents, which are not encouraging and are not fatal

Two real attempts at phone-only replication, and they point the same way.

**Secure Scuttlebutt / Manyverse** is the closest thing anyone has shipped to
this idea: append-only signed feeds, gossip replication, no servers by design.
What actually happened is worth knowing before repeating it — unbounded log
growth, initial syncs measured in hours and gigabytes, and "pubs" that became
de-facto servers because pure phone-to-phone gossip could not keep a network
connected. **The failure was not the cryptography. It was replicating everyone's
history to everyone.**

**Briar** works, on phones, with no servers, in the field. It is deliberately
narrower: store-and-forward messaging between contacts you added explicitly, and
**it does not store other people's data**. Its success is the same shape as the
recommendation here.

The lesson from both: **direct replication between named peers works on phones;
being a general storage node for strangers does not.** SSB failed at the second
while trying to do the first; Briar refused the second and shipped.

### 9.7 Durability without a seeder

If nobody runs infrastructure, what holds the data when every phone is off?

- **The recipients do** (§9.3), and there are usually several.
- **A spare phone on a charger.** Because the data is small, the mobile-native
  answer to durability is the same app on an old handset left plugged in, with a
  "keep this device syncing" toggle. No container, no cloud account, no ten
  dollars a month — the thing most people already have in a drawer. This is the
  version of §10.4 that fits the direction.
- **And a plain encrypted file.** A periodic sealed export to whatever the person
  already uses is not a retreat from the design; it is the same ciphertext
  somewhere else, and it is the only backstop that survives everyone's phone
  being lost at once.

### 9.8 The cost of this direction, named

Going server-free is not free, and the price is paid in the one place already
identified as weakest.

> **No servers ⇒ recipients hold full replicas ⇒ there is no fetch to observe ⇒
> the usage record gets weaker, not stronger.**

§6 set out the trade: replicate for availability and lose observability, or fetch
on demand and keep it. **A phone-only swarm has already chosen replicate** — it
has to, because a peer that is asleep cannot serve a fetch. So the audit claim
under this direction is the smaller of the two:

- **What survives:** every grant and withdrawal, who made it, when, and every
  time data left the phone toward whom. Signed, hash-chained, on the subject's own
  device, and not rewritable by a recipient.
- **What does not:** any record of a recipient reading. There is nothing to
  observe and nowhere to observe it from.

This is worth accepting — **availability at 3 a.m. is worth more than a read
log** — but it must be accepted deliberately and described accurately. It also
means the research/cohort case (§6, §10.0) is the *only* one where the full audit
claim holds, since a researcher fetches under grant rather than following. The
two use cases now want opposite transports, and the design should say so rather
than pretend one mechanism serves both.

### 9.9 The commons endpoint is a gateway, not a bigger phone

The temptation is to make the research node just another peer running the same
software. It cannot be, for a reason that has nothing to do with capacity:
**researchers will not run a Holochain conductor or a p2panda node**, and a design
that requires them to has chosen its framework over its users.

So the commons endpoint is an **impedance match**, and that is its whole job:

- **Swarm side** — an ordinary granted peer. It holds a capability per
  participant, per purpose, with an expiry, and it can be revoked exactly like a
  person can. It is *named* and *joined*, not ambient: nobody's data reaches it
  without a grant naming it.
- **Research side** — the formats research actually consumes. CSV, Parquet, or
  whatever the OPEN and OpenAPS Data Commons shapes settle into. Nothing
  downstream of the gateway needs to know a swarm exists.

**The framework choice stops here.** Whichever of §8.2 or §8.4 is taken, it is
taken on the phone side; the gateway speaks it inward and ordinary research
tooling outward. That contains the dependency risk to one side of one box, which
is worth more than it sounds — it is the thing that makes a wrong framework
choice survivable.

**It is also the way *in*, and this is the part worth building for its own sake.**
Traffic to a commons is not one-directional, and the whanau_voice principle P10 —
*show what came out, not just that something happened* — is the strongest product
feature available here:

- **Cohort baselines back to the participant.** "Your overnight variability
  against 400 people on similar settings" is something no individual can compute
  and every individual wants.
- **What your data supported.** Which analysis, which finding, which paper. This
  is the whole of P10 and it is what makes a use record worth reading rather than
  merely honest.
- **The use record itself**, written by the party that did the using, into a log
  the participant holds.

**Withdrawal has to mean something specific here**, and the whanau_voice wording
is right and should be reused verbatim: *nothing new will be done with it.* A
published finding is not recalled; the next run excludes you; and the count of
withdrawals is itself visible on any analysis that rests on a changed basis.

**Three things this must not become.**

- **A seeder by the back door.** A commons that holds everyone's history for
  durability has quietly become the server this design exists to avoid — and
  Scuttlebutt's pubs (§9.6) are what that looks like a year later. It holds what
  a live grant covers, for as long as the grant lasts, and no more.
- **The default route.** If the commons is the easiest peer to reach, it becomes
  the mandatory path for every record ever written — which is precisely the
  single-ingress problem `seeder-architecture.md` §6 spent a whole section
  removing. Personal sharing must work with the commons absent.
- **An identity broker.** It knows a per-relationship handle and nothing more.
  `consent-usage-layer` §6.2 is unambiguous that linking handles is an act the
  person initiates, not a property of the schema, and a research gateway is the
  most tempting place in the whole design to break that rule.

**And it fixes the residual RQ8 problem.** §8.0 left followers as the peers nobody
can make upgrade. A gateway is operated by an identifiable organisation that can
be asked, which means wire-compatibility pressure lands on the one node in the
network that has an administrator.

## 10. What to build, in order

**Direction, taken as given:** use the existing frameworks and contribute
upstream where they fall short, rather than writing a private protocol. §8.0
removed the objection that made the alternative attractive, so this is now the
better plan on its merits and not only on preference.

Two consequences worth stating before the stages. **Fewer lines are written and
more are read** — the work becomes tracking a moving upstream, reading its
internals well enough to extend them, and carrying patches until they land.
And **the gaps are real gaps, not accidents**: they are open on both projects'
own trackers, which means the contributions are wanted, reviewable and unlikely
to be rejected on principle.

### 10.0 — Decide the epoch and the purposes, not the framework

**§8.9 already decided the framework: p2panda.** What is left to decide is
cheaper and more consequential:

| Decision | Options | Consequence |
|---|---|---|
| **Epoch length** | Hour · **day** · week | Bounds how much a revoked reader keeps. A day costs ~180 KB/year in key records for five readers — so choose on revocation granularity, not on cost |
| **Purposes** | `follow` · `clinician` · `cohort` · `quote` | Each is a key tree. Adding one later is cheap; merging two is not |
| **History on join** | Give a new reader prior epoch keys, or not | p2panda's data mode makes this a per-join choice. A clinician gets history; a cohort gets the window they consented to |
| **Flagship** | Research commons · family follower | Decides which surface is built first, not which crypto is used |

**Recommendation unchanged: research/cohort first.** The incumbent is at its
worst there — donation via Open Humans is a one-way irrevocable upload with no
use record — and the commons gateway (§9.9) is the one place the strong audit
claim survives, which is worth building where it is true.

### 10.1 — Emit: the piece neither framework provides

Unchanged from the first draft, and it matters more under this direction, not
less. **Both tracks need the same thing underneath and neither supplies it:** a
canonical, deduplicated, delta-encoded record stream out of the AAPS database.

A `SwarmPlugin` implementing `DataSyncSelector`, with `plugins/sync/xdrip` as the
template (§5) — CGM debounced to one reading per 5-minute bucket, `referenceId IS
NOT NULL` version rows dropped, `deviceStatus` and `apsResults` excluded by
default (§4) — plus a signed, hash-chained local log of what left the phone,
toward whom and when.

**Build this first regardless.** It is framework-neutral, testable against the
`realdata/` snapshots today with no peers and no network, and it is what any
upstream contribution will sit on top of. It is also the piece most likely to be
useful to other people even if the swarm never ships.

### 10.2 — Spike the key layer on that one emit layer

**Done, as the framework-neutral floor.** `tools/seal.py` implements §7.3's
per-recipient wrapping — X25519 + HKDF + ChaCha20-Poly1305, Ed25519 grants — and
demonstrates the property on 47 epochs of real history: a reader revoked two
thirds of the way through gains none of the 16 epochs sealed afterwards and
keeps all 31 they held. Measured overhead is **164 KB/year for five readers
granted throughout**, against the 180 KB §7.2 estimated from first principles.

That was built first deliberately, the way §10.1 was: it demonstrates the
property today rather than gambling the demonstration on an unfamiliar 0.x API,
and it is now **the behavioural specification the p2panda version has to match**.
Anything p2panda does differently is a question about p2panda.

**Checked, and the answer is better than assumed.** `spike/p2panda-seal`
measures 0.7.1 directly: the property holds (a member removed with four epochs
still to come opens none of them and keeps all six it held), `update()` rotates
without a membership change so a per-epoch key is expressible, and **`remove()`
rotates immediately — so revocation is finer than the epoch**, not coarser. The
reference's "they keep the rest of the epoch" is a pessimistic bound.

**What it cost instead** was the history-scoping claim in §8.4, plus two
practical notes: 0.7.1 needs **rustc ≥ 1.96** (an older toolchain silently
resolves to 0.6.1 rather than failing — a trap the Android NDK chain inherits),
and `EncryptionGroup` takes **six generic parameters** whose real implementations
are the integration work this stage calls the gap.

**Corrected.** This stage previously read "spike both tracks" and described a
Holochain DNA with *a private entry type for records and an assigned `CapGrant`
per recipient* — the architecture §7 retracts and D1 settles against. A build
plan that still executes the rejected design is worse than no build plan, so it
is removed rather than softened.

The remaining spike, on the recommended track (§8.9):

- **p2panda**: `p2panda-auth` + `p2panda-encryption` directly, without waiting
  for `spaces`, since the crates work over raw bytes. Replicate a window to one
  named peer; rotate on removal; and demonstrate **the one property worth
  demonstrating first — after revocation the removed reader decrypts nothing
  new, and everything they already held still opens.**

**What is still worth measuring from the Holochain work**, because §8.2 leaves it
as a credible storage layer with a key layer you supply: what an always-on peer
runtime costs on a loop phone — battery, memory, wake behaviour. Nobody has those
numbers, and they apply to *any* always-on peer, p2panda's included. Measure it
against iroh 1.0 under the transport actually being shipped, not against a
conductor nothing is going to run.

**A spike answers questions no amount of reading settles**, which is the whole
lesson of `seeder-architecture.md` §6.1.

### 10.3 — The gaps, and what contributing to them looks like

| Project | Gap | Shape of the contribution |
|---|---|---|
| **p2panda** | Finish **`p2panda-spaces`** — bundle rotation, expiry configuration, credential unification, concurrent auth messages | Already the project's own tracking issue and already the blocker. This is the highest-value external contribution available anywhere in this document |
| **p2panda** | Android bindings and a mobile story | Currently targets desktop toolkits |
| **iroh / any** | Battery and Doze behaviour of an always-on peer under Android background limits | Measurement first, then whatever it implies. Nobody has published numbers, and they decide RQ9 for whichever transport is used |
| **Anywhere** | A frozen-ish wire profile a follower app can hold for a year (§8.0) | The residual of RQ8, and the thing only a downstream consumer will notice |

**Three Holochain gaps were listed here and are removed.** An embeddable
conductor, its Doze behaviour, and *store-and-forward for an offline author* —
the last of which is the personal-server failure mode §7 rejects, listed as a gap
to fix. Contributing to a design this document has ruled out is how a settled
decision gets quietly re-opened by a work plan.

**Two notes on making this land.** NLnet/NGI funds this space and funded
p2panda's group-encryption work specifically, so **the gap-filling is plausibly
fundable in its own right** — which is a better answer to "who pays for this" than
anything in §9.7. And engaging upstream *before* writing the patch is what
decides whether it is merged or carried forever; the whanau_voice advice holds —
extensions age better than forks against a moving codebase.

### 10.4 — Durability without a seeder

Unchanged, and still the answer: recipients hold what they were granted; a spare
phone on a charger runs the same app with a "keep this device syncing" toggle;
and a periodic sealed export to whatever the person already uses is the backstop
that survives every phone being lost at once.

The reason this is a nicety rather than the whole plan is §9.3: under public
ciphertext, every named recipient already holds a complete encrypted replica. (A
paragraph here previously made the backup load-bearing "under the Holochain
track, because private entries have no redundancy at all" — that track is
retracted in §7, and with it the argument.)

### 10.5 — The commons gateway

Built to §9.9. It sits here, ahead of the review gate, because it is the only
stage with an external party in it and external parties set their own timetable —
it was numbered 10.6 and printed in this position, which is how a reader ends up
executing a different order from the one intended.

- **Swarm side**: one granted peer, per-participant capabilities with expiry.
- **Research side**: an export in the shapes OPEN and the OpenAPS Data Commons
  already use, so a researcher's existing tooling works unchanged.
- **The way back in**: cohort baselines and "what your data supported", returned
  to the participant's own log. Build this in the first version, not the third —
  it is the reason anyone joins, and a commons that only takes is the thing
  people have already refused.
- **Withdrawal**: the next run excludes you, the exclusion count is visible on
  anything resting on a changed basis, and the wording is *nothing new will be
  done with it*.

**The gate here is not technical.** It is finding one research group willing to
be the first endpoint, and their answer decides whether §10.0's recommendation
survives contact with reality.

### 10.6 — Gate: external cryptographic review

Unchanged. Anything that touches insulin data and claims revocation deserves one.
Note that this direction *improves* the position: reviewing a contribution to an
audited, peer-reviewed upstream is a far smaller and cheaper exercise than
reviewing a bespoke protocol, and "we extended OpenMLS/p2panda-encryption" is a
much easier sentence than "we wrote our own group key agreement".

## 11. Calling this decentralised, honestly

Structure borrowed from `seeder-architecture.md` §10, which is the right way to
make this claim.

### What would be true

1. **Authority is decentralised.** The subject holds the keys. No holder can
   read. Unqualified.
2. **The phone is the source of truth and needs nothing.** Not a claim
   Nightscout can make: there, the server is where the data lives.
3. **Custody is multi-party and nobody is a custodian.** A seeder breach is not
   a data breach.
4. **Revocation stops new data by key, not by policy.** Strictly better than
   rotating an `API_SECRET`, and unlike that, it is per-recipient.
5. **The grant record is public, signed and tamper-evident.** Neither party can
   later deny what was granted, by whom, over what range, or when it stopped.
   That is more than Nightscout or Open Humans offers, and it is the honest
   substitute for a read log.

### What must never be said

- **"We can show you who read your data."** Under §7 nobody can — anyone may
  pull ciphertext from the swarm and there is nothing to log. Say *"you can see
  every grant, every withdrawal and every time data left your phone"*, and say
  plainly that a reader opening it is not visible.
- **"Delete my data."** A DHT cannot unpublish (§7.4). Say *"nothing new is
  added and no new key is issued"* — which is true — and never the other thing.
- **"Usage is audited."** It is not, for followers who hold a replica (§6). Say
  *"you can see every grant, every withdrawal, and every time data left your
  phone, toward whom"* — and say what that does not cover.
- **"Revoke."** Say *"nothing new will be sent after you stop it."* Withdrawal
  never recalls what someone already has, and P4's insistence on saying so in
  those words is right, because the alternatives are untrue and people find out.
- **"Your data is stored across a distributed network"** while there is one
  seeder. True at §10.4 and not before.
- **"Trustless."** A recipient you named can screenshot, export and forward. No
  protocol reaches that, and any sentence implying otherwise is a lie a person
  might rely on at 3 a.m.
- Anything implying this is a medical device, or that a follower's view is
  suitable for a dosing decision. §5's read-only isolation is a safety property,
  not an implementation detail.
- **"Fully decentralised"** without naming what it costs. §7.4 is the price
  list: permanent publication, unobservable reads, and a leaked key that never
  expires. All three are acceptable; none is invisible.

### The stretch, named

**There is no reason for anyone to run this yet.** whanau_voice has a board, a
funder and a kaupapa; this has an incumbent that works, a community that already
solved 80% of the problem in a way they are used to, and a real setup-friction
disadvantage. The technical case is solid. **The adoption case rests entirely on
§10.0's recommendation — picking the one use case where the incumbent is
genuinely bad** — and
research donation, where consent today is a one-way irrevocable upload with no
use record, is the honest answer to that.

## 12. Open questions

0. **Do the grant records need to be private too?** §7.4 — a signed public record
   saying *"S granted R"* discloses that R is your endocrinologist, and when you
   joined and left a study. The data is encrypted and the social graph is not.
   Blinded recipient identifiers or encrypted grant bodies are both possible and
   neither is free. **This is the sharpest unsolved problem in the design.**

1. **Does MLS tolerate a Delivery Service that is offline half the day?**
   §8.7. The commit chain has to survive a phone in Doze, a flat battery and a
   week in a drawer. This is the one that decides §10.3.
2. **Replicate or fetch** (§6) — needs deciding by use case, not by architecture
   preference, and probably differs per grant type. A design that quietly picks
   one has picked the use case too.
3. **What does a follower do when the subject's phone has been dark for six
   hours?** Nightscout answers this badly (stale data looks like data). A swarm
   must distinguish *"nothing happened"* from *"nothing arrived"*, and
   `consent-usage-layer` §3.2 is right that quiet periods are information.
4. **Cohort re-identification.** A 5-minute CGM trace is close to a fingerprint;
   pseudonymised swarm membership is not anonymity, and the swarm's own topology
   leaks who is in it. This is the question a research ethics committee asks
   first and neither reviewed design has an answer.
5. **Multi-device.** One person with a phone and a spare is the same
   multi-device group problem `p2panda-spaces` exists to solve, and it is not
   optional — loop phones get replaced, and this project's own history has the
   receipts.
6. **Who holds the keys when the subject cannot?** Diabetes has minors and has
   emergencies. whanau_voice's delegate model (decisions 1 and 11 — *"recognise
   the delegate in the model now; build self-service delegation later"*) is the
   right posture and is harder here, because the delegate for a child is
   permanent and the delegate in an emergency is unplanned.
