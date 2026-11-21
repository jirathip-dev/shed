# ADR: self-update from main's releases, and a serve handover that keeps the terminals (#161)

## Summary

Decision: serve hands over by replacing itself in place: same process id, so every terminal stays its child; the socket and the open terminals pass to the new build through the kernel, scrollback is replayed from the logs on disk, clients reconnect. The updater follows main's head only, asks serve "can this build take over?" before it swaps, and never restarts serve cold.
Slices: (1) a terminal can be detached and picked up again with its screen; (2) serve's handover; (3) the self-update command, its 5-minute job, pause and resume, doctor row, reconnecting commands; (4) docs.
Open questions: (1) a build that passes the check but crashes right after taking over ends that host's plain terminals (ACP sessions survive): accept (recommended), or add a keeper process, about 120 more lines? (2) Slices 1 and 2 need nothing from the release issue #160: build them first as their own issue (recommended), or all in #161?

**Status:** proposed. Shapes, boundaries, statecharts and slices: [README.md](README.md).

## The question

A restart of `shed serve` ends every terminal that is not `--keep` and not ACP, because serve is the
only holder of each PTY's master and the parent of each program. The issue wants a binary swap every time
main moves, with no lane dropped. So: how do the listening socket, the PTY masters, the children, the
screens and the ACP sessions get from the old build's process to the new build's, when the repo forbids
`unsafe` and allows `exec` in one file? And how does the updater pick, check and install a build so that
any failure leaves the host as it was?

What exists already: ACP bridges hold a copy of their master and lend it back after a restart (ADR 0003),
with `hand_over` and `take_over` passing one fd over a Unix socket in safe code; `Terminal::adopt` starts
a terminal from a master and a pid; the history log restores a screen; the bridge reconnects.

## Options

How serve hands over:

| | option | verdict |
|---|---|---|
| A | **Exec in place, once.** Drain, detach the terminals, queue the fds, exec the new binary on the same pid | chosen |
| B | Drover's two steps: start a copy of the new binary on the listeners, drain, exec in place, the copy leaves | A PTY has one reader and a child has one parent. The copy would need every master and screen, then give them back: three transfers where A has one. Drover's copy exists to answer HTTP while requests drain; here the listener's backlog covers the gap, about a second |
| C | A new process takes the fds and the old one exits | launchd and systemd watch the old pid: its exit starts a second serve, and systemd ends the unit's whole cgroup, the terminals included. The children lose their parent and their exit statuses |
| D | Split for good: a small PTY holder that never updates, and a front end that does | The holder becomes the part that cannot be updated without dropping lanes, and terminal ownership is rewritten for it |

A keeps the one step of drover's handover that matters to launchd (the exec in place) and replaces
"start the new binary first" with a preflight: the old serve runs the new binary's `serve --preflight`,
which must start, read the live state file and name a format it can inherit, before anything is frozen.

How the fds reach the new image without `unsafe`:

- By number, with close-on-exec cleared: turning a number into an fd is `unsafe`.
- Reopened through `/dev/fd/N`: on Linux that opens a new PTY for a master, and fails for a socket.
- **Queued with `SCM_RIGHTS` on a socket pair whose other end becomes the new image's stdin** (chosen).
  std hands out stdin as a safe fd, the two helpers exist, and the fds wait in the kernel across the
  exec (measured, README). Nothing is cleared, nothing is numbered, nothing leaks to a child.

How a screen crosses:

- The `Screen` struct serialised: its layout becomes a contract between builds.
- The log tail alone: modes set before the 8 MiB cut are lost. A lost bracketed-paste mode makes `send`
  type a multi-line prompt line by line. A program that was resized shows a different grid.
- A resize signal so programs repaint: it adds a copy of the screen to every scrollback on every update.
- **The log tail, then a repaint string** (chosen): the old screen writes its visible rows, pens, cursor
  and modes as VT text. It is exact, and VT text needs no version.

Where the target comes from:

- GitHub's "latest release": ordered by publish time, so two CI runs finishing out of order step back.
- The commits API: 60 requests an hour for an address without a token; five hosts behind one use it up.
- **`git ls-remote` for `refs/heads/main`, then the release by its tag** (chosen): no quota, no token, no
  JSON. The tag is built from that sha alone, so a `build-staging-…` tag cannot be named.

Order and undo:

- Swap, hand over, roll back on failure: a second path, exercised only when something is already wrong.
- **Check, then swap, then hand over, with no undo** (chosen): `handover.check` runs the staged binary's
  preflight before the swap. After the swap the binary is one that passed checksum, `--version` and
  preflight, so it stays; a serve too busy to drain is asked again by the next pass.

## Decision

1. **Handover is exec in place.** `handover(build)` answers after the check, then serve drains, freezes,
   queues the listener and the masters, and execs `<its own path> serve … --inherit`. Every step before
   the exec is undone by picking the terminals up again in the same process.
2. **The state file is the manifest.** What a lane would miss is saved there (`notes` and the bridge's
   protocol are added); the freeze adds a `handover` section (format, event `seq`, one repaint per
   terminal, in fd order) that the new image's first save drops.
3. **Children are not moved.** The pid is unchanged, so the new image waits on each saved pid and records
   real exit statuses. Children that are not terminals (ssh links, `exec` calls) are ended and reaped
   in the drain.
4. **ACP sessions cross like any terminal.** Their masters are in the queue too; the session itself
   never leaves the bridge. A handover is refused while a live bridge speaks a protocol the new build no
   longer serves, and is taken up by a later pass once that session has ended.
5. **Clients reconnect.** The listener is never closed, so a connect during the gap waits. Streams and
   `wait` calls are closed in the drain; `wait`, `events`, `attach` and `watch` reconnect and ask again
   (`events` with `since`, `wait` with the time left).
6. **The updater only stages, checks, swaps and asks.** It reads main's head, fetches `SHA256SUMS` and
   the asset of `build-<sha7>`, verifies the sum and that `--version` names the sha, asks
   `handover.check`, renames into place, asks `handover`. It never stops, starts or signals serve.
7. **`shed install` installs both jobs** (serve and the 5-minute update), with the job texts rendered
   by farrier-service since #238 (golden copies in farrier's `fixtures/units/`). `--pause` is a marker file.

## Consequences

- **The first hop is by hand, once per host.** A serve from before this cannot hand over: the pass says
  `deferred: serve <sha> predates handover; run shed install` and swaps nothing. `shed install` is the
  manual first install of `docs/releases.md` and restarts serve cold, as today.
- **A host can sit deferred.** After a long time offline the new build may not inherit the old format,
  or a days-old ACP session may be two protocols behind. The updater never drops lanes to get past
  that. `shed doctor`'s `update` row names the reason and the fix (`shed install`, or let it end).
- **The gap is about a second** for tens of terminals: each replays at most 8 MiB at about 130 MiB/s,
  in parallel. `docs/protocol.md` gains the rule other clients need: a closed connection means
  reconnect and ask again.
- **Exit statuses survive a handover.** After a crash and restart an adopted ACP terminal still ends as
  exit 0, as today.
- **A crash right after the exec is not covered** (open question 1). Preflight proves the binary
  starts, reads this state file and knows this format; it cannot prove the rest. The plain terminals
  would get a hangup; ACP sessions would be adopted by the restarted serve. On Linux a keeper would also
  need the unit to stop killing its cgroup when serve dies, which is not so today.
- **A failed exec is survivable but not clean.** The old image picks its terminals up and serves on,
  but std may have reset its signal settings on the way, so it logs the failure and asks for a restart
  at a quiet moment. Preflight has just run the same file, so this takes a file vanishing in between.
- **Output is not lost, and one stray character is possible.** Output written during the gap waits in
  the PTY. `detach` waits up to 100 ms for the PTY to go quiet, so a cut inside an escape sequence
  takes a program writing without pause at that instant.
- **Two methods join the protocol**, so the two protocol docs regenerate (`just generate`). Whether
  that bumps `PROTOCOL` is #159's rule. Any client on the socket may call them; a handover to the same
  build is a restart that drops nothing, and the tests use exactly that.
- **The staged binary is renamed into place, never written over the live file**: macOS checks a
  running binary's signature per file. The folder of the binary must be writable by the user.
- **Hot files** (`shed-server/src/lib.rs`, `registry.rs`, `shed/src/main.rs`, `shed-proto/src/lib.rs`)
  get registration lines and two saved fields; the behaviour is in `handover.rs` and `update.rs`.
  `acp.rs`, 88 lines under its size limit, loses the two fd helpers. The docs are a new
  `docs/self-update.md`, not a section of #160's `docs/releases.md`.
- **cooee and drover** change nothing here. shed's own version-mismatch hints stop recommending
  `cargo install`.
- **The cap** has 490 lines of headroom and this is about +530 (README, Slices).

## `.drover/arch.toml`

No lines to add. This issue turns no shared list into a folder and adds no generated file. The two
protocol docs it changes are already under `generated`, and `runner.rs` is already the one file where
`std::process` is allowed, which is where `replace` goes.
