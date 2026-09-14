# Running a peer

A peer holds a share of other people's sealed records so that somebody's phone
can be asleep and their data still reachable. **It can read none of it** — it is
granted nothing, and there is no setting that would change that.

The deal runs both ways, which is what the project is named after: *ayni*, the
reciprocity where you give because you will need, and somebody else gives
because they will too.

## What it costs you

* **Disk.** Roughly 78 MB a year per subject at a five-minute sensor, and about
  **329 MB** at the one-minute sensor a Libre 3 actually produces. There is no
  pruning yet: storage only grows. Watch the figure at the end of each line.
* **A little bandwidth**, and a connection that stays open.
* **Being visible as a pool member.** Who participates is not secret. What you
  hold is unreadable, but that you are there is public — see `docs/decisions.md`
  D19 and D30.

## Installing

Download the archive for your platform from a `peer-v*` release, unpack it, and
put the binary somewhere on your `PATH`:

| | binary goes | service file |
|---|---|---|
| Linux | `~/.local/bin/diaswarm-peer` | `systemd/diaswarm-peer.service` → `~/.config/systemd/user/` |
| macOS | `/usr/local/bin/diaswarm-peer` | `launchd/nz.diaswarm.peer.plist` → `~/Library/LaunchAgents/` |
| Windows | `%LOCALAPPDATA%\Programs\diaswarm\` | run `windows/install-task.ps1` |

Then:

```sh
# Linux
systemctl --user enable --now diaswarm-peer
loginctl enable-linger "$USER"      # headless: keep it running when logged out
journalctl --user -fu diaswarm-peer

# macOS
launchctl load ~/Library/LaunchAgents/nz.diaswarm.peer.plist
tail -f /tmp/diaswarm-peer.log
```

**`loginctl enable-linger` is the step people miss.** Without it systemd stops
your user's services when you log out, so an "always-on" carrier runs only while
you are logged in — which on a server is rarely.

## Running it by hand first

```
$ diaswarm-peer
diaswarm-peer 0.1.0 — /home/you/.local/share/diaswarm
  node 122dc922…
  adopting up to 4 new subject(s) a pass; it can read none of them
pool 4 · carrying 4 (+3) · 463 operations · pushes yes · 28.4 MB
```

`diaswarm-peer --help` explains the rest. `--adopt 0` stops it taking on new
strangers — read what `--help` says about that before using it, because
carrying for strangers is what keeps *who reads whom* ambiguous.

## It is not a server and has no privilege

Both service files are **user** units on purpose. The peer needs no root, no
ports opened by hand, and one directory under your home. If a sandbox line in
`diaswarm-peer.service` ever stops it starting, delete that line rather than the
sandbox.
