# Running shed serve as a user job

`shed serve` is the per-user server: it owns the terminals, the state file and the socket. Run it as a
user job so it starts at login and comes back after a crash:

```sh
shed install     # writes the job file for this binary and prints the line that loads it
shed doctor      # job file, loaded, socket, version, log: `ok` or `FAIL … fix: <line>`
shed uninstall   # prints the lines that unload the job and remove its file
```

shed writes the file but never loads or unloads it; run the printed lines. A job file with other
contents is reported, not replaced, unless you pass `shed install --force`. The files below are the
same jobs to copy by hand.

| | Socket (mode 0600) | State file (mode 0600) |
|---|---|---|
| macOS | `~/Library/Caches/shed/shed.sock` | `~/Library/Application Support/shed/state.json` |
| Linux | `$XDG_RUNTIME_DIR/shed.sock` | `$XDG_STATE_HOME/shed/state.json` (default `~/.local/state`) |

`shed serve --dir DIR` puts both in `DIR` instead. Only one server answers per socket; a second one
exits with an error.

## macOS (launchd)

[`dist/launchd/shed.plist`](../dist/launchd/shed.plist): set the binary's absolute path in
`ProgramArguments` (launchd does not expand `~`), then:

```sh
cp dist/launchd/shed.plist ~/Library/LaunchAgents/dev.shed.serve.plist
launchctl bootstrap "gui/$(id -u)" ~/Library/LaunchAgents/dev.shed.serve.plist
launchctl bootout "gui/$(id -u)/dev.shed.serve"   # stop and unload
```

## Linux (systemd)

[`dist/systemd/shed.service`](../dist/systemd/shed.service): set `ExecStart` to your binary, then:

```sh
mkdir -p ~/.config/systemd/user
cp dist/systemd/shed.service ~/.config/systemd/user/
systemctl --user daemon-reload
systemctl --user enable --now shed.service
loginctl enable-linger "$USER"   # optional: keep it running while logged out
```

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
