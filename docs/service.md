# Running shed serve as a user job

`shed install` also installs the job that keeps shed on main's latest release; see
[self-update.md](self-update.md).

`shed serve` is the per-user server: it owns the terminals, the state file and the socket. Run it as a
user job so it starts at login and comes back after a crash:

```sh
shed install     # writes the job files for this binary and loads a job that changed or is not loaded
shed doctor      # job file, loaded, socket, version, log: `ok` or `FAIL … fix: <line>`
shed uninstall   # unloads both jobs and removes their files
```

farrier-service renders, loads and removes the jobs; each verb is safe to run twice. A job file with
other contents is reported, not replaced, unless you pass `shed install --force`. The texts it writes
are farrier's golden copies in [`fixtures/units/`](https://github.com/jirathip-dev/farrier/tree/d34a7bb8388ef183c981dc01483634a87f0d3495/fixtures/units) (`dev.shed.serve.*`,
`dev.shed.update.*`).

| | Socket (mode 0600) | State file (mode 0600) |
|---|---|---|
| macOS | `~/Library/Caches/shed/shed.sock` | `~/Library/Application Support/shed/state.json` |
| Linux | `$XDG_RUNTIME_DIR/shed.sock` | `$XDG_STATE_HOME/shed/state.json` (default `~/.local/state`) |

`shed serve --dir DIR` puts both in `DIR` instead. Only one server answers per socket; a second one
exits with an error.

## macOS (launchd)

`shed install` writes `~/Library/LaunchAgents/dev.shed.serve.plist` and `dev.shed.update.plist`, logging to
`~/Library/Logs/dev.shed.serve.log` and `dev.shed.update.log`:

```
$ shed install
dev.shed.serve: written
dev.shed.update: written
```

## Linux (systemd)

`shed install` writes `~/.config/systemd/user/dev.shed.serve.service`, `dev.shed.update.service` and
`dev.shed.update.timer`, and removes the older `shed.service` and `shed-update.*` units (on macOS, the
`shed` and `shed-update` plists). A `shed self-update` pass does the same for an old-named updater: it loads
`dev.shed.update` first, then drops `shed-update.*`; until then `shed doctor` fails `update` with
`fix: shed install`. To keep the server running while logged out:

```sh
loginctl enable-linger "$USER"
```

## Out of memory

An out-of-memory kill ends only the process it picks. The unit has `OOMPolicy=continue`, so when the
kernel kills one terminal's child (a compiler, say), the server and every other terminal keep running.
If the server itself is killed, `Restart=always` brings it back and the terminals are restored as under
Restarts. `shed install --force` rewrites a unit from before this policy and reloads it.

With the job installed, `--autostart` starts the job (`systemctl --user start dev.shed.serve.service`,
`launchctl kickstart`) and waits for its socket; it never starts a server of its own. `shed doctor`
flags a server that answers while the job is not loaded: it runs outside the job's restart policy.

## Restarts

The state file keeps every terminal's name, labels, cwd, command, state and exit status, written
atomically (temp file, then rename). When the server starts again, terminals run with `keep` come back
as a shell (`$SHELL`, else `/bin/sh`) in the cwd they were started in; their task never runs again.
ACP terminals (`run --acp`) keep running while the server is down and are taken back when it starts
(see [ADR 0003](decisions/0003-acp-bridge.md)); one whose bridge ended meanwhile is `exited`. The others are
listed as `exited` with no exit status. `shed hello` exits 1 when the running server is another build.

The file is written compactly at most every 500 ms, so a killed server loses at most the last half second;
it is written at once on a normal stop, and before `run`, `kill` and `rm` answer. An exited terminal is
dropped from the list `$SHED_PRUNE_AFTER` seconds (default a day) after it ended; its log stays on disk until the server next starts.
