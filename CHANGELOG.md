# Changelog

All notable changes to this project are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project uses
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Every build carries its git sha: `shed --version`, `shed hello`, the server's ready line and `shed doctor` show
  `<semver> (<sha>)`. Client and server exchange a protocol number in `hello`; a mismatch (or an older server
  that sends none) fails at once with `client <sha> / server <sha>: protocol <a> vs <b>; restart shed serve`.
  `shed doctor` fails when the running server is another build than the binary, and `shed install` (and
  `scripts/install.sh` when the job is installed) restarts the server.
- `shed setup [<agent>…] [--dry-run] [--remove]` adds hooks that report an installed agent's state through
  `shed state` (a no-op outside shed) to its config file, keeping every other entry; `shed doctor` lists per
  agent whether they are in. Claude Code's are in `~/.claude/settings.json`; see `docs/hooks.md`.
- Parent and child terminals: `shed run` inside a terminal (`$SHED`) records it as the child's `parent`
  (`--parent T` sets it, `--no-parent` clears it); `get`, `ls --json` and every event show it, and
  `ls --parent T` lists a terminal's children. When a parent's program ends or is killed its children are
  killed too, unless started with `--outlive-parent`. `shed wait --children all|any [T]` waits until no child
  is working, or for the next child that reports `done`, `blocked` or `error`, or exits. With `--notify` a
  child that blocks, errors or ends types one `[shed] <name> …` line into its parent (a prompt for an ACP
  parent), held while the parent reports `working`.
- `shed read --events [--json]` (`read` with `events`, paged by `before` and `n`): an ACP terminal's transcript
  as events, each with its `seq`; `tool_call` events carry ACP's tool `kind` when the agent sends one.
- An ACP terminal's screen (`attach`, the `shed watch` preview) draws its events readably: the prompt bold, one
  line per tool call with a status glyph, approvals, the plan as a checklist and the reply's markdown styled.
- One view of every machine: the server holds an SSH link to each configured machine (`shed connect` over
  `ssh`, retried every 5 s, backing off to 60 s), and `ls`, `events`, `get`, `read`, `send`, `keys`,
  `approve`, `cancel`, `kill`, `rm`, `state` and `model` work on `<machine>/<name>`; `run --on M` goes over
  the link. Every machine's events merge into the one stream under its `seq`, so `--since` resumes across a
  dropped link with none lost. `machine ls --json` and the `machine.links` method show each link (`up`,
  `reconnecting`, `down`, since when, why). Another version there is refused naming both. With no link up,
  `--on` uses one `ssh` per call as before.
- ACP agents hibernate when idle: after `$SHED_ACP_HIBERNATE_AFTER` seconds (default 600) with no turn and no
  waiting permission request, shed stops the agent's processes and reports `hibernated`; the next send starts it
  again and loads its session (`session/load`) before delivering it. `run --acp … --no-hibernate` keeps one
  running. Ten idle fake agents: 197,248 KiB awake, 77,568 KiB hibernated; claude-agent-acp: 502,700 KiB to
  8,064 KiB, waking in about 2 s.
- `shed run --acp … --mcp <file.json>` (the `run` method's `mcp_servers`): MCP servers in ACP's shapes (stdio,
  `http`, `sse`) passed to the agent in `session/new` and `session/load`. `http` and `sse` need the agent's
  `mcpCapabilities`, else the terminal ends as `error` naming the server; an unknown or malformed entry is
  refused by `run`. Env and header values stay in memory: the `session` shows them as `***` in its new
  `mcp_servers`, and a resume needs the list again.
- ACP slash commands: `available_commands_update` sets the `session`'s new `commands` (`[name, description,
  hint]`), one `session` event per change. Run one as a prompt `/name …`.
- cooee: pin shed at this release for `RunParams.mcp_servers` and `AcpSession.mcp_servers` / `commands`.
- ACP on the official async SDK (`agent-client-protocol` 5.0.0, on a tokio runtime the bridge owns): every ACP
  message is a typed SDK message, and an unknown `session/update` kind is shown and skipped. Events go to the
  server as notifications, batched, so a 1,000-update turn reaches a socket client about 3× sooner.
- `shed run --acp <id> --resume <session>` (the `run` method's `resume`) loads a session with `session/load`
  when the agent advertises `loadSession` (else the terminal ends as `error`). The replayed history's events
  carry `"replay": true`. A new `session` event, also kept as the terminal's `session` (saved across restarts),
  gives the session id, the current `model` and the `models` (`[id, name]`).
- `shed model <t> <id>` and the `acp.model` method switch the model through the agent's `model` config option
  (`session/set_config_option`; SDK 5 has no `session/set_model`).
- cooee: pin shed at this release for `run.resume`, `acp.model`, the `session` event and `TerminalInfo.session`,
  and `Event.replay` / `EmitParams.replay`.
- The ACP throughput bench (`scripts/bench-acp.sh`, CI job `bench-acp`): 1,000 updates through the bridge at the
  base and the head, failing over 10% slower, plus the head's 1,000-update replay time. Beside it a shed-only
  bench: the fake agent writes 10,000 canned updates at once (`fake_acp.py --canned`), timed per update through
  the bridge and the server, median of 9, also failing over 10% slower.
- `shed watch [--label k=v] [--on M]`: a full-screen live list of the terminals (a state dot, name, fleet,
  issue and role, age; grouped by machine and fleet; an exited one stays dim for 60 s) beside a read-only
  preview of the selected one's screen, at most 10 frames a second. ↑/↓ or j/k select, `/` filters by name
  or label, Enter attaches (`Ctrl+\` comes back to the same selection), `q` quits. Below 80 columns the
  preview is hidden. Plain ANSI escapes and raw mode; no TUI crate.
- `screen.watch {terminal, since}` streams a running terminal's screen: the full screen, then only the
  rows that changed, as runs of text with their colours, bold, italic, underline and inverse, plus the
  cursor, the scrollback length and the size; at most 20 updates a second. `terminal.input`,
  `terminal.keys` (`ctrl-x` works as `ctrl+x`) and `terminal.resize` (the watched terminal by default)
  send input; `terminal.readonly` makes a terminal read-only for one connection, which then gets
  `READONLY` for any input, keys or resize to it.
- `shed machine stats [--on M] [--since T]` and the `machine.stats` method: load per CPU, memory pressure,
  swap used, free disk on the state volume, CPUs, uptime and the terminals by state. The server keeps one
  sample a minute for 24 h (`stats.json` beside the state file); `--since` returns them. With `--on`, a
  machine that gives no answer within 5 s is reported offline with when it was last seen (exit 1).
- `shed install [--force]` writes the per-user job for `shed serve` (macOS: `~/Library/LaunchAgents/dev.shed.serve.plist`,
  RunAtLoad, KeepAlive, a PATH with `~/.cargo/bin` and `~/.local/bin`, logs to `~/Library/Logs/shed.log`;
  Linux: `~/.config/systemd/user/shed.service`) and prints the line that loads it; a file with other contents
  is refused without `--force`. `shed uninstall` prints the lines that unload and remove it. `shed doctor`
  checks the job file, that it is loaded, the socket, the server's version and the log, with a fix line
  for each failure (exit 1).
- Other agents' state escapes: Jan's OSC 9999 (`{"agentType":…,"state":…}`) sets the reported state
  (`waiting` reads as `blocked`) with `state_source: "jan-osc9999"`; garbled or unknown payloads are
  ignored. Each agent is one row in one table.
- `shed events --on <machine>` survives a dropped SSH link: each terminal seen gets a state line noted
  `machine: reconnecting`, the link is retried every 5 s (backing off to 60 s after 5 failures), a
  `machine: connected` line follows once it is back, and the stream resumes after the last `seq`. Names in
  that stream are `<machine>/<name>`.
- `shed serve` prints one line, `shed ready <version> <socket>`, to stdout once its socket accepts
  connections (it replaces the `listening on` stderr line). `serve --detach` and `run --autostart` wait for
  it and say whether the server exited first or was not ready within 10 s.
- ACP turns no longer hang: a reply or adapter error with one of the agent's known auth-failure lines (for
  `claude-acp`: `Failed to authenticate`, `OAuth session expired`) ends the turn as `error`
  (`auth: <the line>`) after a `reply` with `stop_reason` `auth_error`, and a turn that hears nothing from
  the agent for `$SHED_TURN_IDLE_TIMEOUT` seconds (default 120; a waiting permission request aside) ends
  as `error` (`no response`).
- Bounded terminal history: each terminal keeps at most 5,000 lines and 8 MiB of scrollback (was 10,000
  lines), dropping the oldest whole lines. Raw output is appended to `history/<name>.log` beside the state
  file, cut back to its last 8 MiB once it passes 16 MiB. After a server restart, `read` of a lost
  terminal and a kept terminal's new shell start from only the log's last 8 MiB (an incomplete UTF-8
  character at the cut is skipped). `just bench` also reports the history throughput.
- Machines over SSH (M3): `shed machine add <name> <ssh-target>`, `ls` and `rm` (kept in `config.toml`
  under `[machines]`), a global `--on <machine>` that runs the terminal commands (including `attach`,
  `wait` and streaming `events`) on that machine through one `ssh <target> shed connect` (key login only:
  `BatchMode=yes`, a 10 s connect timeout, `ControlMaster=auto` where ssh supports it), and `shed connect
  [--autostart]`, which carries the protocol over stdin/stdout. A shed of another version on the machine
  is refused with the exact `cargo install` command to match it. With `--on`, `run` needs an absolute
  `--cwd`.
- ACP client mode (M2c): `shed run --acp <agent> [--policy allow|deny|ask]` runs any Agent Client Protocol
  (v1) agent as a structured terminal. `send` is a prompt, `approve` answers permission requests (state
  `blocked`), `cancel` (or `keys <t> esc`) cancels the turn, the state is `working` then `done` with the stop
  reason, `read` renders the JSONL transcript (prompts, one line per tool call, replies), and `events` gains
  `turn`, `text`, `tool_call`, `plan`, `mode`, `approval_request`, `approved` and `reply`. `shed agents
  [add|rm]` installs agents from the ACP registry, pinned. The protocol gains `emit`; a terminal carries
  `acp`. See docs/acp.md.
- Events and resources (M2b): `shed events [--label k=v] [--since N]` streams `started`, `state`,
  `exited {code}`, `killed` and `resources {rss_mb, cpu_pct}` as JSON lines, each with a `seq`; `--since`
  replays from a ring of the last 1,000. `shed serve --sample S` (default 2) samples every process
  tree's memory and CPU; `resources` fires when either moves by more than 10 %. `ls` gains RSS and CPU
  columns, `get` a `resources` field, and `shed top` lists terminals by memory. The protocol gains
  `events`.
- State reporting and waiting (M2a): `shed state [<t>] working|blocked|done|error [--note]` (inside a
  terminal `$SHED` names it), the OSC escape `ESC ] 7501 ; state=<s>[;note=<n>] BEL` parsed from the
  output and kept off the screen, `shed wait <t> --until <state>… [--output RE] [--timeout S]` that blocks
  server-side and exits 0 on a match, 1 on exit and 124 on timeout, and `shed run --idle-after S` (report
  `idle` after S silent seconds until the program reports). The protocol gains `state` and `wait`; a
  terminal carries `reported` and `idle_after`.
- The `shed` CLI (M1c): `run`, `ls`, `get`, `read`, `send`, `keys`, `kill`, `rm` and `attach` (raw mode,
  `Ctrl+\` detaches, window resizes forwarded) as a thin client, `--json` on every command, a global
  `--dir`, `shed serve --detach` and `shed run --autostart`. The protocol gains `resize` and `attach`.
- `shed serve` (M1b): the per-user server on a Unix socket (mode 0600) speaking newline-delimited
  JSON-RPC 2.0 (`hello run ls get read send keys kill rm`), unique names and `k=v` labels, a state file
  (atomic writes) that brings `keep` terminals back as shells in their cwd after a restart, and a
  same-version handshake (`SHED_VERSION`). `shed-proto` holds the types; `docs/protocol.md` is
  generated from them. launchd/systemd user-job templates in `dist/` (docs/service.md).
- `shed-core` terminals (M1a): programs on real PTYs, a VT screen model with bounded scrollback,
  named keys, and process-tree kill (SIGTERM, a grace period, SIGKILL) with per-tree RSS/CPU.
- The workspace skeleton (M0): `shed hello`, `--help`, `--version`, and every CI gate.
- ACP usage carries the turn's and the session's input, cache-read, cache-write and output tokens (`usage.turn`
  and `usage.total` in `get` and the `usage` event) when the agent's prompt responses report them, as
  `claude-acp`'s `_meta.quota` does; `shed top` shows the last turn's cache-read share as `CACHE`.
- A provider record's `[cache]` gives the prompt cache's lifetime on a plan and on an API key and the variables
  that mean an API key; an ACP lane hibernates after 600 s or that lifetime, whichever is shorter (unless
  `$SHED_ACP_HIBERNATE_AFTER` is set), `get` shows `session.cache_likely` (`warm` or `cold`) while idle and
  `shed top` shows it as `CACHE-LIKELY`; `shed model` after a turn says the switch drops the prompt cache.
- Agent facts are data: each agent's limit and login-expired lines, hook table, login, status and logout
  commands, config-folder and settings variables and effort flags are one TOML provider record, built in
  from `crates/shed-server/src/providers/<name>.toml` and overridden key by key, or added, by `<name>.toml`
  in `providers/` beside the config file, without a rebuild. `shed doctor` names a bad user file, which is
  skipped. `shed agents --json` and the new `agents` method list each record (with its `source`) beside the
  installed adapter (now under `installed`); `agents.match {id, text}` applies an agent's limit and auth
  rules to a caller's text. See docs/acp.md.
- cooee and drover: read program facts from `shed agents --json` in place of their own copies.
- Screen rules for PTY agents: a terminal labelled `agent=<agent>` checks its provider record's `[[rules]]`
  (a state, a priority, the last N screen lines and texts to match) on screen changes, at most every 250 ms;
  a rule above the agent's own report outranks it while its screen shows. Claude Code's folder-trust prompt
  reads `blocked` (`permission`). `get` says where a state came from: `reported.from` is `reported`,
  `rule:<id>` or `silence`. See docs/hooks.md.
- `shed top` and `shed ls` show an ACP terminal's agent state (`working`, `done`, `hibernated`, …) as STATE.
  `top` splits an ACP terminal's memory into AGENT (the bridge, adapter and agent as the turn began) and WORK
  (what it started), kept as `resources.agent_mb` and `work_mb`, and ends with a totals line. `shed top <t>
  [--full]` lists that terminal's processes over 50 MB: pid, memory, CPU, age and the command's name (the
  whole command line with `--full`).
- `shed run --acp … --warm` keeps one ready spare session per recipe (agent, command, folder, policy, MCP
  servers, meta, size): the next `--warm` run of that recipe takes it, renamed to its name and labels, with
  `session.ready_ms` 0, and a new spare starts. Spares are listed as `.warm-N`; `$SHED_WARM_MAX` (default 2)
  caps them and `$SHED_WARM_IDLE` (default 600 s) kills one nobody took.
- `shed ls --json` and `get` show each ACP session's start-to-ready time (`session.ready_ms`) and its first
  prompt's time to the first reply chunk (`session.first_token_ms`); `shed agents --json` gives each agent's
  `medians` of both over its last `$SHED_MEDIAN_OF` (default 20) sessions.
- ACP permission requests by id: `approval_request` carries its `id` (the agent's JSON-RPC request id) and ACP's
  `toolCall` as `call` (without `content`); `shed approve <t> <option> --request <id>` (the bridge's
  `{"approve": …, "request": …}` line) answers that request, and without it the oldest as before. `tool_call`
  carries ACP's `locations` (paths) and `raw_input` (left out over 4 KiB). All additive.

### Changed

- `shed serve` saves its state file at most twice a second, compact and off the registry lock, and at once on
  shutdown; `run`, `kill` and `rm` still answer only once the change is on disk. An ACP terminal's slash
  `commands` and `models` are no longer saved (the agent resends them). Exited terminals are dropped from the
  list a day after they end (`$SHED_PRUNE_AFTER` seconds of `shed serve`); their logs stay until the next start.
- Release builds use fat LTO, one codegen unit and a stripped binary: 4.0 MB instead of 8.0 MB on macOS arm64.
- `shed events --on M` with no held link is one plain stream over `ssh`: the client-side reconnect and its
  `machine: reconnecting` / `machine: connected` lines are gone; the server's held link does the reconnecting.
- The ACP path is faster per update: the bridge reads a text chunk straight from the agent's JSON instead of
  the SDK's types, writes its notifications without a `Value` between, and the server parses each request line
  once, straight into a request, and reads params without copying them.
- The server runs on one tokio runtime: connections, streams and watchers are tasks instead of threads (20
  terminals and 50 `events` clients: 24 threads instead of 143). Events go to every client from one shared log,
  each client's new events written as one batch. The ACP bridge lost its two glue threads. The CLI and the
  protocol are unchanged.
- Server responses list object keys in field order instead of alphabetically (the SDK turns on serde_json's
  `preserve_order`). The JSON is the same.
- Renamed from berth to shed: the crates (`shed`, `shed-core`, `shed-proto`, `shed-server`), the binary,
  the socket (`shed.sock`), the state and config dirs, the service units (`dev.shed.serve`,
  `shed.service`) and the docs. Terminals get `$SHED`. The `hello`
  carries `product: "shed"`, so a berth client or server is refused with `SHED_VERSION` instead of
  half-working. OSC 7501 is unchanged.
- `shed-proto` no longer turns on serde_json's `preserve_order` or pulls in `schemars` for its users: the
  JSON Schema derives and `openrpc()` sit behind its `schema` feature, off by default. `shed` and
  `shed-server` turn both on, so `shed schema`, `rpc.discover` and the wire JSON are unchanged.

### Fixed

- One server per state file: `shed serve` holds a lock (`state.lock` beside the state file) for its life, so a
  second server is refused even when the first one's socket file is gone, instead of restoring the kept
  terminals twice and taking the socket. A killed server's lock goes with it. Two `run --autostart` at once
  share the one server that wins.
- `shed rm` also removes an ACP terminal's transcript, and `shed serve` removes the logs and transcripts of
  terminals its state file no longer names.
- `shed machine add|rm` writes the config through a temp file of its own, so writers running at once no longer
  fail on a shared one.
- `shed kill` of a terminal running an interactive shell no longer waits out the 2 s grace: a kill sends SIGHUP
  100 ms after SIGTERM (each with SIGCONT), so a shell ends at once and `shed serve` stops quickly. A program
  that handles SIGTERM still runs its handler first; one that ignores both is still killed after the grace.
