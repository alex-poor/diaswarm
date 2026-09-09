# Decisions

What is settled, why, and what evidence would reopen it. Reasoning lives in
[feasibility.md](feasibility.md); this is the index so a decision is not quietly
re-litigated six weeks from now.

**`D1`–`D9` here are decisions. `RQ1`–`RQ10` in feasibility.md §8 are the
requirements frameworks are judged against.** Both were `D` until a review found
that `D3` meant *"audit is given up"* in this document and *"data survives for
years"* in the other one — in two documents that cite each other on every page.

---

### D1 · Records are public ciphertext, not private and pull-only

**Settled.** A swarm exists to make data available to your people *whether or not
your phone is on*. Anything that answers reads from the author's device — a
Holochain private entry, a remote zome call — makes the subject a personal
server, which is the thing a distributed store exists to fix.

So records are published and replicated, therefore encrypted, therefore access is
**key custody**. "Named recipients" is a statement about keys, not about storage.

*Reopens if:* the flagship use case turns out to need read-auditing more than
availability. See D3 — that is the trade being made.

### D2 · Framework is p2panda

**Settled**, on the three layers a design needs: storage swarm, **key layer**,
grant records. p2panda is the only candidate supplying all three.
`p2panda-encryption` *data mode* — shared group key, rotated on removal, joiners
given prior secrets deliberately — is the requirement described in the library's
own terms.

Holochain is a good storage swarm with **no key layer for public entries** (cap
grants gate zome calls, not DHT reads). Hypercore's read key is irrevocable.
Meadowcap gates sync, not rest.

*Reopens if:* `p2panda-spaces` stalls past the point of usefulness. Fallback is
OpenMLS for the key layer — audited, an RFC, and its Delivery Service problem
mostly dissolves because the subject's phone is the only writer of its own data.

⚠️ **One claim this decision rested on is now measured false.** feasibility.md
§8.4 said the library could "withhold history from a researcher while granting it
to a clinician", because history-on-join is a per-join choice. It is not:
`add()` welcomes a joiner with the entire secret bundle, and a member added at
the end of the spike opened every epoch including six from before it existed.
**Time-scoped grants need one group per window**, which is more group management
than the design assumed. This does not reverse D2 — the key layer is still the
only one on offer — but it moves work into the integration.

✅ **Cadence checked 2026-09-08, and that condition is not met — the opposite.** v0.7.1
was published **21 August 2026, eighteen days ago**, with 0.5.0/0.6.0/0.7.0 in
January, May and July of the same year. **`p2panda-spaces` is published** at
0.7.1, and its tracking issue is closed. What remains open — documentation,
key-bundle auto-rotation, concurrent encryption messages, group promotion and
demotion — are open items on a released crate.

This decision was previously annotated with the reverse conclusion, from a date
in feasibility.md that was off by a year. **The recommended track is in better
shape than the documents said**, and OpenMLS stays a fallback rather than
becoming live. Re-check again before committing to the sealing work, since a
crate three weeks old is a moving target in a different way.

*Watch:* proxy re-encryption (Umbral/TACo lineage). Uniquely allows granting
access to already-published data without the subject's device participating,
which is what a commons enrolling at scale would want. Too immature to build on;
TACo is being forked and relaunched H2 2026 — **which is now**, so this is a
thing to go and look at rather than a thing to wait for.

### D2a · The key layer ships as the reference construction, not p2panda

**Settled 2026-09-09, amending D2.** D2 chose p2panda for the key layer. The
spike that tested it (`spike/p2panda-seal`) confirmed the property holds and that
revocation is *finer* than the epoch — and also that **`add()` cannot scope
history on join**: a member added at the end opened every epoch, including six
from before it existed. Time-scoping therefore needs one p2panda group per
window.

**D11 made that the flagship's problem, not a research edge case.** *"A clinician
gets the last 90 days"* is a window. Under per-recipient wrapping a window is
free — it is simply which keys you wrapped. Under p2panda it is a group to
create, join, rotate and revoke, per grant.

So the epoch-and-wrap construction is what ships: `crates/diaswarm-core::seal`,
ported from `tools/seal.py` and **byte-compatible with it**, verified by sealing
in one language and opening in the other in both directions.

**This is a deferral, not a rejection.** The key layer sits behind one seam —
`epoch_key`, `wrap`, `unwrap` — so p2panda can replace it without touching the
record path, the plugin, or the file format. What changes is that it is no longer
on the critical path, and the project no longer waits on someone else's 0.x.

*Reopens if:* multi-device (§12.5) or a care team with several writers turns out
to need real group agreement, which is what a CGKA is actually for and what
per-recipient wrapping does badly. That is the case to watch, and it is not the
flagship.

⚠️ **Still not reviewed cryptography.** X25519 + HKDF-SHA256 +
ChaCha20-Poly1305, Ed25519 for grants, composed by hand. D2's fallback argument
— *"we extended an audited upstream" is an easier sentence than "we wrote our
own"* — now cuts against this choice, and stage 10.6's review gate matters more
because of it, not less.

### D3 · Audit is given up, deliberately

**Settled and uncomfortable.** Public ciphertext means anyone can pull it and
nobody can log a read. What survives is a signed, public, tamper-evident record
of **grants** — who was given what, over what range, and when it stopped.

That is less than "usage is audited" and more than the incumbent offers. It must
be described accurately, in the words in feasibility.md §11, because a person may
act on it.

*Exception:* the research commons (D5) fetches under grant and is an identifiable
organisation under an agreement, so the strong claim holds there — socially, not
cryptographically.

### D4 · Epoch keys, one UTC day each

**Settled, and now specified.** spec/records.md §5.1 freezes it:
`epoch = floor(t / 86400000)`.

**UTC, deliberately, and it costs something.** An epoch must have the same
identity on every device — a local-midnight boundary is ambiguous across travel
and DST, and two peers disagreeing about which epoch a record belongs to is a
correctness problem in a replicated store. The price is that away from UTC the
boundary falls inside the waking day (in NZ, near noon), which makes *"they keep
the rest of the epoch"* harder to say plainly to the person deciding whether that
is acceptable. A wording problem in one place against an ambiguity problem
everywhere.

*Reopens if:* asking real people about revocation granularity (below) shows the
"rest of the epoch" promise is unsayable at a noon boundary. The fix would be a
per-subject fixed offset, not local time.

A content key per epoch, wrapped to each live grantee and published beside the
data. Granting starts the wrapping; revoking stops it. This buys time-scoped
access for free (give a researcher one year's keys), and it bounds what a revoked
reader keeps to **one epoch**.

**Cost is now measured, not estimated.** The reference snapshot spans 49 epochs:
245 wraps for five readers, about **24 KB of key records beside 1.66 MB of
data** — 179 KB/year against the 180 KB/year §7.2 predicted. So cost is
irrelevant to the choice, as claimed. **Choose on revocation granularity.** Nobody has yet asked a person whether "they keep up to 24 more
hours" is acceptable, and that is a question for people, not for this repo.

### D5 · The commons is a gateway, not a bigger phone

**Settled.** Researchers will not run a p2panda node, so the endpoint is an
impedance match: an ordinary granted peer on the swarm side, ordinary research
formats (CSV, Parquet, the OPEN / OpenAPS Data Commons shapes) on the other.

**The framework choice stops at that box**, which is what makes a wrong framework
choice survivable.

It is also the way data comes **back** — cohort baselines, and what your data
supported. Build that in the first version: a commons that only takes is the
thing people have already refused.

*Must not become:* the default route, an identity broker, or a holder of history
it was never granted.

### D6 · Canonicalisation is part of the protocol

**Settled.** A dump of the AAPS database is not a person's history — it is the
history plus its own edit log. Version rows (`referenceId IS NOT NULL`) and
retracted rows (`isValid = 0`) are removed at the emit boundary, never
downstream, because a consumer that takes the tables at face value over-counts
insulin and carbs and biases every model fitted on them.

Loop telemetry (`deviceStatus`, `apsResults`) is excluded: two thirds of the
file, no clinical content, and the tables most likely to hold something nobody
meant to share. There is no flag to re-include it — the record vocabulary is
closed, and the flag that claimed to do this never did anything.

**Units are normalised at the same boundary, for the same reason.** AAPS stores
glucose values and temporary targets in mg/dL but profile blocks in whichever
unit the user set, so an un-normalised stream carries the same kind of quantity a
factor of 18 apart in two different records, separated only by a flag. That is
the failure spec/records.md §2 exists to prevent, and it was live in the emitter
until it was measured. A unit the emitter does not recognise is passed through
untouched *and* marked, because a visible gap beats a plausible wrong number.

### D7 · The AAPS plugin is read-only, structurally

**Settled and non-negotiable.** A `DataSyncSelector` drains a queue outward.
Nothing in that shape can write back into the loop, and nothing here may acquire
the ability. Anything that can influence dosing is inside the medical device's
blast radius and inherits its entire risk posture.

### D8 · A 0.x dependency is acceptable

**Settled**, reversing the assumption inherited from the whanau_voice work
(*"you can freeze a three-opcode protocol you wrote, and you cannot freeze
someone else's 0.x crate"*).

That rests on peers who never update. AAPS has none: `VersionCheckerPlugin`
applies expiry as a loop constraint — `maxIob.set(0.0, …)` — so an expired build
**stops dosing**. Every actively looping install is on a recent release.

*What survives:* followers are not loopers and have no expiry mechanism. That
compatibility burden lands at the gateway (D5), which has an administrator.

### D9 · Extend upstream, do not fork

**Settled.** Gaps get contributed: `p2panda-spaces` (rotation, expiry, credential
unification, concurrent auth messages) is already the project's own tracking
issue and already the blocker. NLnet/NGI funds this space and funded p2panda's
group-encryption work specifically, so the gap-filling is plausibly fundable in
its own right.

Engage upstream *before* writing the patch. Extensions age better than forks
against a moving codebase.

### D12 · Followers are Android. iOS is out of scope

**Settled by the project's owner, 2026-09-08.** iOS cannot hold a background
socket, so an iPhone cannot be a peer. Every route to serving it — push, or a
relay that queues for an absent peer — ends at Apple's push service.

**Declining is more consistent with the design than serving it badly.** The whole
claim is that nobody in the middle can read or withhold anything (§9.4); putting
APNs in the delivery path would contradict it for the flagship's most visible
surface. feasibility.md previously called this the biggest single risk and
proposed answering it before designing the follower app. It is instead accepted.

*What it costs, plainly:* in most countries a large share of the people a subject
might share with are on iPhones, and they are excluded entirely. **This cannot be
recommended as a general replacement for Nightscout while that holds**, and any
description of it must say so rather than implying broad follower support.

*Reopens if:* adoption beyond Android becomes a goal, or someone is willing to
run a queuing relay and be named as the party in the middle.

### D11 · The flagship is personal sharing, not research

**Settled by the project's owner, 2026-09-08.** The point is **privacy and
sovereignty**: a person shares their own history with friends, family and
clinicians, revocably, with nobody in the middle. Research donation stays a good
use case and is not the first one.

This resolves an inconsistency rather than creating one — feasibility.md §6
already called "a parent seeing their child's CGM at 3 a.m." the flagship, while
§10.0 recommended research/cohort first because that is where the incumbent is
worst and where the strong audit claim survives. Both of those remain true; both
were the wrong reason to pick the first surface.

**What it costs, taken deliberately:**

- **The strong audit claim does not apply here.** Followers hold replicas, so
  what survives is grants and egress, never reads (D3). The §6 trade —
  availability at 3 a.m. over a read log — is now being made in the flagship, on
  purpose, and §11's wording is the primary promise rather than a footnote.
- **iOS followers become the biggest single risk**, not a conceded gap
  (feasibility.md §9.5). A parent with an iPhone is the flagship, and cannot be a
  peer.
- **Windowing lands in the main path.** "A clinician gets the last 90 days" is a
  window, and D2's measurement says p2panda cannot scope history on join.
- **The incumbent to beat is Nightscout, not Open Humans** — and on the
  subject's own side this is *easier* on convenience, not harder: Nightscout
  means a ~$5/month host plus a mandatory database, and a forced migration
  whenever a platform drops a tier. This means a toggle in an app already
  installed. The convenience losses are on the **follower** side — iOS above all
  — and feasibility.md §11 has the table.

*Reopens if:* the sovereignty argument turns out not to move anyone, and a
research group turns out to be willing to fund or host. That is an adoption
finding, not a technical one, and it would change which surface is built next
rather than anything below it.

### D10 · The sealing reference is per-recipient wrapping; p2panda is what ships

**Settled.** §7.3 already said per-recipient wrapping is "the floor and it is
sufficient" at five readers. `tools/seal.py` is that floor, built
framework-neutrally for the same reason `canon.py` was: it demonstrates the
property on real data today instead of gambling the demonstration on an
unfamiliar 0.x API, and it prices the construction — 164 KB/year for five
readers, against the 180 KB §7.2 estimated.

**It is a reference, not a competitor.** The shipping key layer is
`p2panda-encryption` data mode (D2). What this buys is that the port has a
behavioural specification to match rather than a paragraph, and that anything
p2panda does differently becomes a visible question about p2panda.

**Measured, 2026-09-08** (`spike/p2panda-seal`): p2panda 0.7.1 gives the same
property, and its revocation is **finer** than the epoch — `remove()` rotates
immediately rather than at the day boundary. So the reference's promise is the
pessimistic one, which is the safe direction to be wrong in.

*Reopens if:* the reference stays more expressive in a way that matters. It
already is in one way — **per-recipient wrapping scopes history by which keys you
wrapped, and p2panda's `add()` cannot scope history at all** (see D2). If windowed
research grants become the flagship, that difference decides whether this
construction rides on top of p2panda's transport rather than being replaced by
its group.

⚠️ **Not reviewed cryptography.** Standard primitives composed by hand. Stage
10.6 exists to catch exactly this, and nothing here should be described as
reviewed until it has been.

---

## Open, and blocking nothing yet

One line per open question in [feasibility.md §12](feasibility.md), in its
numbering, plus one this index adds. **This list previously held five of the
seven and renumbered them**, which is how an open question stops being tracked.

| §12 | Question | |
|---|---|---|
| **0** | **Do grant records need to be private too?** A signed public record saying *"S granted R"* discloses that R is your endocrinologist, and when you joined and left a study. **The data is encrypted and the social graph is not.** It pulls directly against the tamper-evidence D3 offers as the substitute for a read log | *The sharpest unsolved problem in the design* |
| 1 | **Does MLS tolerate a Delivery Service that is offline half the day?** The commit chain has to survive Doze, a flat battery and a week in a drawer | Decides whether D2's fallback is real |
| 2 | **Replicate or fetch**, per grant type rather than per architecture. A design that quietly picks one has picked the use case too | The D3 trade, applied case by case |
| 3 | **What does a follower see when the phone has been dark six hours?** A swarm must distinguish *"nothing happened"* from *"nothing arrived"* | Nightscout answers this badly |
| 4 | **Cohort re-identification.** A 5-minute CGM trace is close to a fingerprint | The question an ethics committee asks first |
| 5 | **Multi-device.** A phone and a spare is the problem `p2panda-spaces` exists to solve, and it is not optional — loop phones get replaced | Blocked on the same gap as D2 |
| 6 | **Delegation.** Diabetes has minors and has emergencies. The delegate for a child is permanent; the delegate in an emergency is unplanned | |
| — | **Who operates the commons**, and does being a named, revocable peer actually change what an ethics committee thinks? That is the claim this design makes to that audience and it has never been tested on one | Not in §12; the gate on D5 |
