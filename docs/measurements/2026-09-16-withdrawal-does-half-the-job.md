# "Stop sharing with" does half the job, silently — 2026-09-16 20:20

**Found by trying to use it.** A revoke was issued on the loop phone against a
real granted reader, through the app's own UI. The plugin logged:

```
swarm: no keys identity on record for 68cbe8cd9549aa48… — the core withdrawal
stands, but if they are a keys member this did NOT remove them.
Retry once they have handed over (within 30 minutes).
```

**All seven readers on that phone have `"keys": null`.** So this is not one
unlucky reader — **withdrawal cannot complete for any of them.**

## What actually happens

A reader is two memberships (`KnownReader`'s own comment says so):

* one in the **segment vault**, wrapped per segment — removed correctly;
* one in the **keys group**, named by a tag derived from the pair — **not
  removed**, because `keysRevokeReader` takes the reader's *keys identity* and
  the book does not have it.

So the person is out of one and still in the other. If they are a keys member
they go on reading everything sealed from here.

## Why the field is empty

`vaultNoteReaderKeys` records the identity **at grant time**, and only when the
invite carried one — a v3+ invite. Every reader granted before that field
existed, or from an older invite, has `null` forever unless a D27 handover fills
it in. A follower sends one every half hour, so it self-heals *for readers whose
app still runs*. For the rest it never does.

## The part that makes it a consent bug rather than bookkeeping

🔴 **THE LOG WARNS AND THE SCREEN SAYS NOTHING.** The subject taps "Stop sharing
with", sees no error, and reasonably believes they have withdrawn. The plugin
knows it did not and says so only where nobody is looking.

[roles.md](../roles.md) is explicit about what a subject must never be told, and
this is squarely in it: *"❌ 'Revoke' as though it recalls closed days"* is
listed, but silently failing to revoke **at all** is worse than the wording
problem that section was written about.

⚠️ **The code comment is right that "I have no record" is not "there is
nothing".** It cannot assert they are out. But the honest response to that is to
*tell the subject*, not to return quietly.

## What this cost today, concretely

The revoke was an attempt to force an announced rotation, to recover a follower
blacked out by [the unannounced-rotation bug](2026-09-16-the-leak-is-our-own-stall-remedy.md).
It could not: **no keys removal means no rotation means no announcement.** Ayni's
`unreadable` count sat unchanged at 177 across the whole attempt.

## Fixes, in the order they matter

1. **Say so on screen.** A withdrawal that could not complete must not look like
   one that did. The wording already exists in the log; it belongs in the dialog.
2. **A reader with no keys identity should be recoverable**, not permanently
   un-withdrawable. The tag is derived from the pair, so the subject holds half
   of it already — establish whether the identity can be recomputed rather than
   waiting on a handover that may never come.
3. **Until then, the reliable way to force a rotation is grant-then-revoke a
   throwaway** whose invite carries a keys identity, because that one *can* be
   named.
