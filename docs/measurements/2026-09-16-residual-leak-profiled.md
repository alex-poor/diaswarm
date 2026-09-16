# The residual leak, named — 2026-09-16 15:00

heapprofd on phone B, attached to a process **6 h 33 m old** and left running for
20 minutes. That is the condition every earlier capture failed: past warm-up,
past the first hour, in the state the leak actually lives in.

**Two retainers, and neither is iroh.**

| what | net retained, 20 min | allocations | callsites |
|---|---|---|---|
| **tokio broadcast rings** in `p2panda_net::sync` | **43.4 MB** | 1 129 | 14 |
| **SQLite page cache** on sqlx connections | **31.5 MB** | 3 046 | 2 |
| everything else | 2.3 MB | 69 | 4 |
| **`iroh` `pending_open_paths`** | **absent** | — | — |

⚠️ **THE MAGNITUDES ARE SAMPLED AND RUN ABOUT 2× HIGH.** heapprofd attributes
~91.8 MB net over the window; `dumpsys` measured the process actually going
285 500 → 325 931 KB, so **+40 MB**. Trust the *ranking*, not the totals.

⚠️ **AND THE WINDOW ITSELF RAN HOT.** +40 MB in 20 min is +2.0 MB/min against
the +0.52 MB/min steady rate measured over hours. `block_client: true` makes the
app wait on the profiler. **Do not quote +2.0 as the leak rate.**

---

## 1. The bigger half is a per-topic ring that never drops

```
tokio::sync::broadcast::channel::<TopicLogSyncEvent<...>>
  -> Vec<Mutex<Slot<TopicLogSyncEvent<...>>>>
  -> RawVecInner::with_capacity_in
  <- p2panda_net::sync::actors::topic_manager::TopicManager<...>
```

`p2panda-net-0.7.1/src/sync/actors/manager.rs:41` sets
`FROM_SYNC_CHANNEL_BUFFER = 1024`, and line 317 allocates one
`broadcast::channel(1024)` per topic. Each ring is **~176 KB**, and the profile
shows roughly **70 of them retained per 20 minutes** — about 3.5 a minute that
are allocated and never freed.

**It is not us re-subscribing.** `Replicator::stream` guards with
`streaming.lock().insert(topic_bytes)` and returns early, so each topic is
subscribed exactly once on our side. The churn is inside p2panda-net: either
topic managers are torn down and rebuilt, or `state.sync_receivers` keeps
receivers whose rings can never be reclaimed.

✅ **COUNTED, AND THE TOPIC SET IS FLAT.** `logcat` on the same process, across
the capture window and after it:

```
14:50:47  diaswarm: keys carrying 5 log(s)
14:51:28  diaswarm: keys carrying 5 log(s)
...
14:56:47  diaswarm: keys carrying 5 log(s)
```

**Five topics, never moving, while ~3.5 rings a minute are allocated and
retained.** So this is not the cost of carrying more subjects — the rings are
being re-created for topics that already exist, and the old ones are not
dropped. That is an upstream bug in p2panda-net, not a property of the swarm
getting bigger.

It also fits the cadence: a sync pass logs every ~40 s, and 5 topics re-armed
every pass is the right order of magnitude for 3.5 rings a minute.

**So the next step is upstream-shaped**, which is where it belongs: establish
whether `ToSyncManager::Create` is reached for an already-subscribed topic (its
guard returns early, so something must be evicting `topic_manager_map`), or
whether `state.sync_receivers.insert(topic, from_sync_rx)` is replacing a
receiver whose ring is still held by the spawned `TopicManager`. Either is
reportable to p2panda with exactly the evidence above.

## 2. The smaller half is the page cache, measured properly this time

```
pcache1Alloc <- sqlite3Malloc <- getPageNormal <- pcache1FetchStage2
  <- getAndInitPage <- moveToChild <- sqlite3VdbeExec <- sqlite3_step
  <- sqlx_sqlite::statement::handle::StatementHandle::step
  <- sqlx_sqlite::connection::worker::ConnectionWorker::establish
```

The same stack the page-cache theory was built on, and **withdrawn** — but it was
withdrawn for the right reason and is being readmitted on different evidence. It
was never the *burst*; the burst was iroh. On a six-hour-old process with the
bursts patched out it is 31.5 MB per 20 minutes, second only to the rings.

**Checked, and the cap is not the missing piece.** Audited every `sqlite://` URL
in `crates/`: the app opens exactly two file-backed stores — `keys.sqlite`
(`diaswarm-android/src/lib.rs:467`) and `addressbook.sqlite`
(`diaswarm-net/src/swarm.rs:199`) — and **both go through
`open_bounded_store`**, which sets `cache_size = -2000`. The rest are dev
binaries and tests, which never run on a phone. And p2panda-net opens no store
of its own; it is handed ours (`KeysReplicator::keys(store, …)`), so there is no
third uncapped pool hiding behind it.

**So this half is bounded, and probably is not a leak at all.** 2 MB a
connection × up to 16 per store is a ceiling, and the allocating frame is
`ConnectionWorker::establish` — a *newly established* connection warming its
(capped) cache. The sampler watched the pool swing 11 → 20 during the soak, and
~15 connections' worth of 2 MB is the right order for the 31.5 MB attributed
here once the ~2× sampling inflation is taken off.

⚠️ **That is an inference, not a measurement.** What settles it is the pool
count logged beside the memory — `tools/mem-curve.sh` now records `sqlx`, which
it should have from the start. If the count sits at 20 while `pcache1Alloc`
keeps retaining, the ceiling is not holding and this becomes a real leak again.

---

## 3. How to symbolise this build, which is not obvious

🔴 **`traceconv symbolize` resolves nothing here, and silently.** It reported
"6284 frames could not be symbolized" and wrote a 0-byte file.

The native half is **stored uncompressed and mapped straight out of the APK**, so
every one of our frames has the mapping name `…/base.apk`, not
`libdiaswarm_android.so` — and the `.so` carries no GNU build-id, only
`.note.android.ident`, so perfetto has nothing to match on.

**What works:** `stack_profile_frame.rel_pc` is already the offset into the `.so`
and needs no adjustment. Feed it straight to the NDK symbolizer:

```sh
llvm-symbolizer -e libdiaswarm_android.so -f -C --output-style=GNU <rel_pc>
```

The binary must be **the one installed**. Phone B's installed APK carries a
42 MB unstripped `.so` (291 569 symbols, `.debug_info` present) — pull it with
`adb pull $(adb shell pm path …)` and unzip, rather than trusting a local build
to match.

## 4. Two things worth recording that were not the question

* **Phone B has `swap=0`.** The loop phone had 204 MB in zram, which is what made
  its PSS curve understate by ~2×. Phone B's historical curves do **not** have
  that error — the correction in the sibling note applies to the loop phone.
* **Ayni ran 6 h 52 m without the foreground service being killed**, against the
  ~6 h `dataSync` limit recorded in [[relay-drops-overnight]]. ⚠️ It was
  USB-attached and charging throughout, which is exactly the condition that can
  change FGS accounting, **so this does not overturn that finding** — it is a
  reason to re-test overnight on battery before believing either version.
