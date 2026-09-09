# diaswarm

Sharing diabetes data from FOSS loop apps — AAPS first — with **named people you
choose**, revocably, without an operator in the middle.

> Your phone keeps writing exactly as it does now. Nothing about the loop
> changes. What is added is the ability to hand a partner, a clinician or a
> research cohort a key to some of it, and to take that key back.

**Status:** pre-POC. One tool works and is tested
([`tools/canon.py`](tools/canon.py)); nothing else exists yet. The architecture is settled and written down — start at
[docs/feasibility.md](docs/feasibility.md).

---

## The architecture in five lines

1. Records are **published as ciphertext** to a swarm of peers. Holders cannot
   read what they hold.
2. Access is **who holds a key**, never who a server decides to serve.
3. Keys are **per-epoch** (a day) and **wrapped to each live grantee**.
4. **Granting** starts the wrapping. **Revoking** stops it — prospectively, and
   nothing recalls what someone already has.
5. A **research commons** is a separate, always-on peer with its own gateway,
   because researchers will not run a p2panda node.

**Framework: p2panda**, whose `p2panda-encryption` *data mode* is built for
exactly this — a shared group key, rotated on member removal, with joiners given
prior secrets deliberately. Gaps get contributed upstream rather than forked.

## What this is honest about

The whole design is one trade, and it is worth reading before anything else:

| | |
|---|---|
| **You get** | Data available to your people when your phone is off. Per-recipient revocation enforced by key. A signed, tamper-evident record of every grant |
| **You give up** | Any record of someone *reading*. Publication is permanent — there is no "delete my data". A leaked key never expires |

Language that must never be used, and the true version of each, is
[docs/feasibility.md §11](docs/feasibility.md). It is not decoration: someone may
make a 3 a.m. decision on what this says.

## Repository

```
docs/feasibility.md    The assessment. Architecture, frameworks, costs, plan
docs/decisions.md      What is settled, and what would reopen it
spec/records.md        The canonical record stream — the wire contract
crates/diaswarm-core   spec/records.md in Rust + sealing + the vault. One implementation
crates/diaswarm-net    Moving a vault between peers, over iroh
crates/diaswarm-core/src/bin/diaswarm.rs   the desktop side: seal, grant, revoke, read
spike/p2panda-seal     What p2panda-encryption 0.7.1 actually does, measured
tools/canon.py         AAPS SQLite → canonical records. Works today
tools/seal.py          Canonical records → epochs, sealed and wrapped per grantee
tools/mkfixture.py     A synthetic AAPS database, so the above runs with no real data
tools/test_canon.py    Tests for the filters spec/records.md rests on
tools/test_seal.py     Tests for the one property revocation has to have
tools/port-session.sh  Make a Claude Code session resumable from this repo
```

The assessment is also published as a single mobile-readable page:
**<https://claude.ai/code/artifact/8430821f-2724-402e-a5c5-881b8ea1f3ff>**. It is
a rendering of `docs/feasibility.md`; the repo is the source, and the page has to
be republished when the source moves.

## Resuming the design session

This work started in `~/projects/camaps`, and Claude Code keys sessions to the
directory they ran in. The originating session has been ported here:

```sh
claude --resume 6a0f1ea9-a0d9-497d-a32f-1b0e33127fd2
```

The copy is a snapshot. **Re-run `tools/port-session.sh <id>` at the end of any
session in the old directory**, or the tail of it is lost. `tools/port-session.sh`
with no argument lists candidates, newest first.

Project memories were seeded too — the swarm architecture, the AAPS database
hazards, how to pull a fresh snapshot off the phone. The camaps loop memories
(pump protocol, keybox, Hovorka) were deliberately left behind.

## Try the one thing that works

`canon.py` turns an AAPS database into the canonical stream and reports what it
dropped and why. It is read-only, needs no network, and is the piece no framework
provides.

```sh
tools/canon.py /path/to/androidaps.db --stats
```

On a 48-day snapshot of one real loop, in full — every line the tool prints:

```
  kept
    cgm          11,974
    tbr           6,141
    bolus           729
    carb            170
    event            73
    target           29
    profile          12
    TOTAL        19,128

  dropped
    invalid           2   isValid = 0
    version      25,173   referenceId IS NOT NULL
    cgm dup           0   second+ reading in a 5-min bucket

  cgm      272.4/day after debounce over 44.0 days of CGM
          (95% of the 288 a 5-minute sensor can produce)

  size       1.66 MB ndjson     0.19 MB gzip   over 48.5 days
  year       12.4 MB ndjson      1.5 MB gzip   projected
```

Two spans, deliberately: the records cover 48.5 days, the CGM in them covers 44.0,
because the first reading arrives 4.6 days after the first record of another kind.
Rates are quoted over the span of the thing being rated.

**1.5 MB a year, compressed.** That number is why none of the hard parts of this
are storage problems: a decade of one person's history fits in a phone's spare
change, and a swarm member can hold many people's ciphertext without noticing.

Real snapshots never enter this repo — `.gitignore` refuses `*.db` and
`*.ndjson`. Copy the `-wal` alongside the `.db` or you will silently read stale
data.

**Which means nobody else could run any of this.** So there is a fixture:

```sh
tools/mkfixture.py /tmp/fixture.db && tools/canon.py /tmp/fixture.db --stats
tools/test_canon.py
```

`mkfixture.py` writes a synthetic AAPS database containing one of each hazard —
version rows, retracted rows, a row that is both, two CGM readings in one bucket,
a carb entry with no duration beside one with a real zero, profiles in mmol/L and
mg/dL and in a unit nobody recognises, and a table on an older schema. The tests
assert what §3 of the spec claims, and they have been checked against a
deliberately broken canonicaliser to confirm they fail when it is wrong.

## The property, demonstrated

`seal.py` cuts a canonical stream into UTC-day epochs, seals each under its own
content key, and wraps that key to every live grantee. **Granting starts the
wrapping; revoking stops it.** There is no delete step and no message to anyone —
a revoked reader is simply one the loop no longer wraps for.

```sh
tools/canon.py snapshot.db -o stream.ndjson && tools/seal.py stream.ndjson --demo
```

On the same 48-day history, sealed a day at a time with the revocation landing
two thirds of the way through:

```
  sealed  47 epochs, 19,128 records, one day at a time
  partner revoked at epoch 20663, with 16 of 47 epochs still to come

  what each reader can open, at the end
    partner    31 of 47 epochs  follow
    clinic     47 of 47 epochs  clinician
    cohort      7 of 47 epochs  cohort

  the property
    nothing new          0 epochs gained after the stop   PASS
    nothing recalled     31/31 previously held epochs still open   PASS
    per-recipient        clinic unaffected: 47 of 47 epochs   PASS
    revocation bites     partner holds 31 of 47, blind to 16   PASS

  on the wire
    key records       7.6 KB   85 wraps of 92 bytes
    five readers      164 KB/year   granted throughout, against §7.2's estimate of 180
```

**Both halves of that are the promise.** A design where the revoked reader keeps
reading is a broken revocation; one where their existing copy goes dark is
claiming a recall it cannot perform, which
[§11](docs/feasibility.md) says never to claim.

It needs `cryptography` (X25519, ChaCha20-Poly1305, Ed25519), and it is
**not reviewed cryptography** — standard primitives composed by hand, which is
what stage 10.6's review gate exists to catch. It is also not a swarm: it writes
files, and where those bytes go is a later stage.

## Next

Stage order and reasoning are in
[docs/feasibility.md §10](docs/feasibility.md). Immediately:

- ~~**Freeze the record shape**~~ — **done.** [`spec/records.md`](spec/records.md)
  is v1 as of 2026-09-08: UTC-day epochs, an in-band schema version, `tdd`
  removed, and ordering that two implementations agree on. Streams declare the
  version they conform to, so a v2 is detectable rather than discovered.
- ~~**Epoch sealing**~~ — **done as the framework-neutral reference.**
  [`tools/seal.py`](tools/seal.py) demonstrates the property on real history and
  prices it at 164 KB/year for five readers, against §7.2's estimate of 180.
- ~~**Port it to `p2panda-encryption`**~~ — **done and measured.**
  [`spike/p2panda-seal`](spike/p2panda-seal/FINDINGS.md): the property holds, and
  revocation is *finer* than the epoch. But `add()` cannot scope history on join,
  so a windowed grant needs its own group — and *"a clinician gets the last 90
  days"* is a window.

**The flagship is personal sharing — friends, family, clinicians**
([D11](docs/decisions.md)). Privacy and sovereignty is the point; research
donation is a good second. So, next:

- **Followers are Android; iOS is out of scope** ([D12](docs/decisions.md)).
  Every route to an iPhone ends at Apple's push service, and this design's claim
  is that nobody in the middle can read or withhold anything. A real exclusion,
  stated rather than engineered around.
- **The AAPS plugin**, a `DataSyncSelector` sibling of `plugins/sync/xdrip`
  (1,109 lines there as the template, verified against the checkout). Read-only
  out of AAPS, always. The record logic is not written twice: the plugin is a
  thin Kotlin shell over [`crates/diaswarm-core`](crates/diaswarm-core) via the
  NDK, and that crate is asserted byte-identical to `canon.py` over 19,129 real
  records.
- **Group-per-window**, now that it is in the main path rather than a research
  edge case.

## Licence

AGPL-3.0 — see [LICENSE](LICENSE). The AAPS plugin links AGPL code and must be;
the rest follows it for now.

**This is not settled.** Anything intended to land in p2panda should be written
as a patch to p2panda under *their* Apache/MIT terms, not carried here — and if
a shared core crate turns out to be worth other people using, it needs a
permissive licence and that decision has to be made deliberately before there is
history to relicense.

## Not a medical device

Nothing here participates in dosing, and the AAPS plugin is structurally
read-only. A follower's view is not suitable for a treatment decision. Data can
be stale, absent, or withheld by design, and the interface must say which.
