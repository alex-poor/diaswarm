# A subject a reviewer can actually follow

*2026-09-16*

## The problem this closes

Ayni is a follower. It shows somebody else's glucose, and it needs a subject to
grant it. The only subject that exists is the AndroidAPS add-on, which is
deliberately not distributed — running a loop means building a medical device
from source you have read.

So the path for anyone evaluating Ayni ends immediately:

1. Install it.
2. See *"Not following anyone yet"*.
3. Stop.

An F-Droid reviewer hits this. So does anybody deciding whether to trust the
app, and any developer with one phone. Nothing in the app is broken; there is
simply nothing they can do with it.

🔴 **The obvious shortcut is not available.** Granting a reviewer a read on the
loop phone's vault would be a stranger reading a real person's glucose. No
review convenience is worth that, and it is not offered here or in the MR.

## What was built

`diaswarm-peer demo` — a synthetic subject. Invented readings, sealed and
granted and announced through exactly the code a phone runs.

```
diaswarm-peer ~/demo demo --grant <the follower's invite>
```

It prints an invite, then publishes a reading a minute forever.

* **The readings are generated in `demo.rs`** — a daily sine, three meal-shaped
  bumps, and a wobble, clamped to 55–320 mg/dL. Deterministic in the timestamp,
  so a restart continues the curve rather than jumping.
* **One a minute**, because that is what a Libre 3 does. A demo at 5-minute
  cadence would give a follower the wrong idea of what it holds.
* **Every record carries `src: DEMO`**, so nothing downstream — a report, an
  export, a clinician's screen — can mistake it for a sensor.
* **The handle is `demo (not a real person)`** and travels in the invite.
* **`--backfill` seals history on the first pass** (default 6 hours), so a
  follower shows a graph rather than a single dot. A reviewer who sees one
  reading cannot tell a working app from a broken one.
* **It is an ordinary subject otherwise**: its own identity, both vaults, the
  real pool, the real relay. A demo that took a shortcut through the protocol
  would prove nothing about the protocol.

Verified end to end across two peers over the live pool: 240 backfilled readings
plus the ongoing ones, **248 records read by a second peer that had only been
handed the invite**, newest at the current minute.

## Three traps it walked into

### 1. A vault under the wrong name is served to nobody

The first draft put the core vault in `demo-core/`, because that reads well.

`diaswarm_net::answer` resolves every core-vault request by joining the store
directory to the 64-hex subject in the request. A vault under any other name
answers every follower with the same empty manifest a peer gives for a subject
it has never heard of — **a naming mistake wearing the costume of a network
fault.** Nothing in a compile, and nothing in a unit test of the curve, could
see it.

`the_wire_can_find_the_demo_vault_by_subject` asks the wire instead of asking
the code, and fails with `meta.json` against the old name.

### 2. An invite a version ahead of the app is a dead end

The demo emitted `diaswarm:4:` invites, because `with_handle` bumps the version
and the handle is what says *"not a real person"*.

`Invite::parse` refuses a version it does not know — correctly, since it cannot
check a field it has never heard of — and says *"update to use it"*.

**The published Ayni is not this tree.** At the time of writing the F-Droid
metadata builds 0.1.6, whose `invite.rs` tops out at v3. A reviewer handed only
the named invite would have seen a refusal and reasonably concluded the app does
not work — the exact impression this command exists to remove, delivered by the
command itself.

It now prints **both**: the named v4, and a v3 that drops the name and nothing
else. `encode` already picks its version from the fields actually set, so the
fallback is a genuine v3 rather than a v4 with a blank in it.
`the_fallback_invite_is_one_an_older_follower_can_read` is a tripwire: if a
later field bumps the fallback off v3, somebody has to decide whether every
installed follower can still read it, rather than finding out from a bug report.

### 3. Sealing and announcing must not be separable

`keysRotate` returned a `Message` and dropped it, and a follower went dark for
an afternoon. Every function in `demo.rs` that produces an operation publishes
it before returning, and the rotation path prints **"rotated but could not
announce it"** rather than swallowing the failure — rotated-and-unannounced is
the dangerous state, not rotation failing.

## Also

`--grant` takes either an invite or the bare keys identity
`diaswarm-peer identity` prints. Android's `keysGrant` already accepts either,
for the reason that applies here too: a person pasting what their app gave them
should get the grant they asked for, not a lecture about which of two formats it
was. A bare identity opens the keys vault only — it carries no core reader key
to wrap for — and the command says which vaults it actually opened.

## What this does not do

* It does not make the demo discoverable. Somebody has to be given the invite.
* It does not run anywhere yet. Offering it in the MR means running it somewhere
  durable for the review period, which is a separate decision.
* Nothing here changes the loop phone or the AAPS plugin.
