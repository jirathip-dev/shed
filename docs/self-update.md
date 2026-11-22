# Self-update (#161)

shed keeps itself on main's latest release. Nothing else installs, upgrades or restarts it: cooee and drover
only check that the running build is compatible. The one other install path is the manual first install of a
release in [releases.md](releases.md#a-manual-install), followed once by `shed install`.

## The job

`shed install` writes and starts two user jobs through farrier-service ([service.md](service.md)):

| | serve | self-update, every 5 minutes |
|---|---|---|
| macOS | `dev.shed.serve` | `dev.shed.update` (`StartInterval` 300), log `~/Library/Logs/dev.shed.update.log` |
| Linux | `dev.shed.serve.service` | `dev.shed.update.timer` running `dev.shed.update.service` (oneshot), log `journalctl --user -u dev.shed.update` |

## One pass

`shed self-update`:

1. With `<state dir>/update/paused` present, it says `paused since <unix time>` and exits 0.
2. It reads main's head with `git ls-remote <repo> refs/heads/main` and names the release `build-<sha7>` from
   that sha alone, so a `build-staging-<sha7>` release can never be picked.
3. Main's head is this binary's build: up to date, or, when serve still runs the build before, it asks serve
   to hand over.
4. No `SHA256SUMS` on that release yet: it waits (`waiting for build-<sha7>`) and changes nothing.
5. It downloads `shed-<arch>-<os>` to `<binary>.new`, checks its sha256 against `SHA256SUMS` and that
   `<binary>.new --version` ends in `(<sha7>)`. A failure removes `<binary>.new` and exits 1 (`rejected`).
6. It asks serve `handover.check` on `<binary>.new`. A refusal (an older serve, a busy one, an ACP session
   the new build cannot serve) is `deferred`: nothing is swapped, the next pass asks again.
7. It keeps the live binary as `<binary>.prev` (a hard link, for a manual rollback) and renames
   `<binary>.new` over the binary, so a running binary is never written over.
8. It asks serve to `handover` to the new build: the same process replaces itself, every terminal and ACP
   session keeps running with its screen ([protocol.md](protocol.md)), and clients see their connection close.
   A serve too busy to drain is `pending`, and the next pass, now running the new binary, asks again.

Each pass writes `<state dir>/update/status.json` (`at`, `current`, `ok`, `why`) and prints its line only when
it differs from the last one (or on a terminal), so the job's log holds changes only. One pass runs at a time
(`<state dir>/update/lock`). `$SHED_UPDATE_REPO` and `$SHED_UPDATE_RELEASES` point a pass at another git
remote and release URL (the tests use a local repo and a `file://` folder).

```console
$ shed self-update --dry-run
current  e55a367, serve e55a367
target   build-1a2b3c4 (main)
would    fetch shed-aarch64-apple-darwin, check, swap, hand over
$ shed self-update --pause
$ shed self-update --resume
```

`shed doctor`'s `update` row shows the last pass, and fails on a `rejected` or `deferred` one with its reason.
A serve from before the handover cannot take over: run `shed install` once, which restarts it cold.

## Clients

`shed wait` and `shed events` reconnect when a handover closes their connection: `wait` asks again with the
time it has left, `events` with `since` set to the last event it printed. `attach` and `watch` end and are
started again.
