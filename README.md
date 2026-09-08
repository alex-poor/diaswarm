# diaswarm

Sharing diabetes data from FOSS loop apps — AAPS first — with **named people you
choose**, revocably, without an operator in the middle.

> Your phone keeps writing exactly as it does now. Nothing about the loop
> changes. What is added is the ability to hand a partner, a clinician or a
> research cohort a key to some of it, and to take that key back.

**Status:** pre-POC. One tool works ([`tools/canon.py`](tools/canon.py)); nothing
else exists yet. The architecture is settled and written down — start at
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
tools/canon.py         AAPS SQLite → canonical records. Works today
tools/port-session.sh  Make a Claude Code session resumable from this repo
```

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

On a 48-day snapshot of one real loop:

```
  kept
    cgm          11,974
    tbr           6,141
    bolus           729
    carb            170
    TOTAL        19,128

  dropped
    version      25,173   referenceId IS NOT NULL
    invalid           3   isValid = 0

  cgm      246.6/day after debounce (a 5-min sensor can produce 288)

  size       1.66 MB ndjson     0.19 MB gzip   over 48.5 days
  year      12.5 MB ndjson      1.5 MB gzip   projected
```

**1.5 MB a year, compressed.** That number is why none of the hard parts of this
are storage problems: a decade of one person's history fits in a phone's spare
change, and a swarm member can hold many people's ciphertext without noticing.

Real snapshots never enter this repo — `.gitignore` refuses `*.db` and
`*.ndjson`. Copy the `-wal` alongside the `.db` or you will silently read stale
data.

## Next

Stage order and reasoning are in
[docs/feasibility.md §10](docs/feasibility.md). Immediately:

- **Freeze the record shape** in `spec/records.md` — every later stage encodes
  against it, and changing it after a peer exists is expensive.
- **Epoch sealing**, per §7.2: content key per day, wrapped per grantee, and the
  one property worth demonstrating first — *after revocation the reader decrypts
  nothing new, and everything they already held still opens.*
- **The AAPS plugin**, a `DataSyncSelector` sibling of `plugins/sync/xdrip`
  (~1,100 lines there as the template). Read-only out of AAPS, always.

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
