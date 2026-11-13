# ACP agents

shed runs any [Agent Client Protocol](https://agentclientprotocol.com) agent (Claude Code, Codex, Pi, Gemini, …)
as a structured terminal: prompts, tool calls, approvals and replies come back as events and states instead of
screen text. ACP v1 only; v2 becomes an opt-in once it is stable (see [ACP v2](#acp-v2)).

## Install an agent

```sh
shed agents add claude-acp      # or codex-acp, pi-acp, gemini, …
shed agents                     # id, pinned version, distribution, command
shed agents rm claude-acp
shed agents sessions claude-acp # the agent's sessions: id, last activity, folder, title
```

`add` reads the ACP registry (`--registry <url|file>`, default
`https://cdn.agentclientprotocol.com/registry/v1/latest/registry.json`) and installs the version it lists now:
this platform's `binary` entry (downloaded with `curl`, checked against its `sha256`, unpacked under
`agents/<id>/<version>/` beside the state file), else `npx -y <package@version>`, else `uvx <package>`. The
entry's `args` and `env` are kept. The result goes to `agents.json` beside the state file. Nothing upgrades an
agent: `rm` it and `add` it again.

## Run one

```sh
shed run --acp claude-acp --label issue=7 --cwd /home/user/project
shed send t1 "Fix the failing test"
shed wait t1 --until done waiting-approval rate-limited error
shed approve t1 allow-once        # an option id, or a kind: allow_once, reject_once, …
shed read t1                      # > prompt, [tool] title (status), the final reply
shed read t1 --events             # the same events drawn: › prompt, ✓ kind title, the reply styled
shed read t1 --events --json      # each event with its seq; page with --before <seq> --lines <n>
shed cancel t1                    # or: shed keys t1 esc
shed model t1 opus                # one of `shed get t1`'s session.models
shed run --acp claude-acp --resume <session id>   # load a past session (`agents sessions`, `shed get`'s session.id)
shed run --acp claude-acp --mcp servers.json      # with MCP servers (below)
shed run --acp claude-acp --meta '{"k": 1}'       # with session options (below)
shed send t1 "/review src/"                       # a slash command is an ordinary prompt
```

`--policy allow|deny|ask` (default `ask`) answers permission requests: `allow` and `deny` pick the first
allow or reject option at once; `ask` reports `waiting-approval` (note: the tool) until `shed approve`. With a command
after `--`, `run --acp <id> -- <cmd…>` runs that command as the adapter instead of the installed one.

The state is `working` from `send` until the turn ends, and `done` (note: the stop reason, e.g. `end_turn` or
`cancelled`) after; an agent error is `error`; [Lane states](#lane-states) has them all. A turn also
ends as `error` when the agent's login failed (note `auth: <the line>`, see [Accounts](#accounts)) or when
it hears nothing from the agent for `$SHED_TURN_IDLE_TIMEOUT` seconds in the environment of `shed serve`
(default 1800; note `no response`). Anything the agent sends counts: every update (thought chunks, plans,
usage, tool-call progress), any request, and each line it writes to stderr; a permission request waiting for
`approve` does not time out. The default is long because a model thinking at high effort can send nothing
for minutes on a healthy turn, and the watchdog is off while a tool call is `pending` or `in_progress`; an expired login is caught at once by its reply, so the watchdog is only a
backstop for an agent that really hangs. `shed events`
streams `turn`, `text` (reply chunks), `tool_call` (each status change), `plan`, `mode`, `approval_request`,
`approved`, `reply` (the whole reply and its `stop_reason`), `session` (its id, the current `model` and the
`models`, also kept as the terminal's `session`) and `usage` (the agent's `usage_update`: tokens `used` of the
context window's `size` and the session's `cost`, also kept as `session.usage`; `shed top` shows it as
`TOKENS`, e.g. `53k/200k`) next to the usual `started`, `state` and `exited`; see [protocol.md](protocol.md).

`--resume <id>` (the `run` method's `resume`) needs an agent that advertises `loadSession`, else the terminal
ends as `error`. `shed agents sessions <id>` lists the agent's sessions (`session/list`, every page; run in the
current folder) when it advertises `sessionCapabilities.list`, and refuses otherwise; `--json` gives ACP's
session objects. The agent replays the session's history first: those events carry `"replay": true` and rebuild
the transcript, so `read` shows the whole conversation. The session id survives a server restart in the lost
terminal's `shed get`. `shed model` (the `acp.model` method) sets the agent's `model` config option
(`session/set_config_option`; the SDK shed uses has no `session/set_model`).

### Lane states

An ACP lane reports only these states (`shed ls`, `shed get`'s `reported`, `state` events), each from what shed
itself sees:

| state | meaning | source |
|---|---|---|
| `working` | a turn is open and the agent is producing output | ACP stream |
| `running` | a tool call is `pending` or `in_progress` (note: its title), or the turn ended while a process the agent started is still alive (note: `running: <command> (<minutes>m)`) | ACP tool calls, process tree |
| `waiting-approval` | a `session/request_permission` waits for `shed approve` (note: the tool) | ACP stream |
| `rate-limited` | the agent hit its account limit (note: the reset time, when the agent states one) | the agent's limit pattern |
| `done` | a clean turn end and no live process the agent started (note: the stop reason) | ACP stream |
| `error` | the agent failed or exited (note: why) | ACP stream, exit |
| `hibernated` | the agent's process stopped, its session kept; the next send wakes it | shed |

A process counts as the agent's own when it was in the agent's process tree as a turn started (the adapter,
its MCP servers); any other live descendant once the turn ended makes the lane `running`, the one nearest the
agent named in the note, checked every 5 s until it exits. A tool call left open when the next turn starts is
forgotten. `rate-limited` lasts until the next turn starts: shed neither retries, wakes nor continues the
lane, nor switches its account. An agent's limit pattern is data, one entry per registry id in
`crates/shed-server/src/providers/<agent>.rs`:

```rust
static LIMIT: Limit = Limit {
    says: &["hit your limit", "usage limit reached"], // a reply or error line holding one of these
    resets: &["resets "],                             // the reset time follows one of these on that line
};
```

`claude-acp` (`You've hit your limit · resets 3pm` → `3pm`) and `codex-acp` (`You've hit your usage limit. …
try again at 3:05 PM.` → `3:05 PM`) have entries. An agent without one never shows `rate-limited`; its limit
text stays its reply (`done`) or error (`error`). Adding one is a data entry and one line in
`providers/mod.rs`'s list, never a change to `acp.rs`.

### Hibernation

An idle agent hibernates: once its session has had a turn (or was resumed) and has been `done` (or, resumed,
has had no turn yet) for `$SHED_ACP_HIBERNATE_AFTER` seconds in the environment of `shed serve` (default 600; `0`
never), the bridge stops the agent's process tree and reports the state `hibernated` (a `state` event). The
terminal, its session id, model, models and slash commands stay; only the bridge (about 8 MB) is left. The next
send (a prompt, `model`, …) starts the agent again with the same command, folder and environment, loads the
session (`session/load`, its replayed events marked `"replay": true` but not added to the transcript again)
and then delivers it; sends that arrive meanwhile follow in order. The MCP servers stay in the bridge's memory
only. An agent without `loadSession`, or a `run --acp … --no-hibernate` (the `run` method's `no_hibernate`),
never hibernates. Measured on 2026-11-07: ten idle fake agents took 197,248 KiB awake and 77,568 KiB
hibernated (wake 76 ms); one claude-agent-acp 0.100.0 took 502,700 KiB in 7 processes awake and 8,064 KiB in 1
hibernated, and woke to its first update in 1,946 ms (`crates/shed/tests/acp_bench.rs`, `hibernate` and
`hibernate_live`).

`shed rm` of an exited ACP terminal closes its session (`session/close`) when the agent advertises
`sessionCapabilities.close`: the agent's process is gone by then, so shed starts the agent once in the
terminal's folder, sends `initialize` and the close, and stops it (at most 30 s; a failure does not stop the
`rm`). Without the capability only the handshake runs. Hibernation never closes a session; it loads it again.

### MCP servers and slash commands

`--mcp <file>` (the `run` method's `mcp_servers`) is a JSON list of MCP servers in ACP's shapes, which shed
passes to the agent in `session/new` and in `session/load`:

```json
[
  {"name": "files", "command": "/usr/local/bin/mcp-files", "args": ["--root", "."],
   "env": [{"name": "TOKEN", "value": "…"}]},
  {"type": "http", "name": "web", "url": "https://mcp.example/", "headers": [{"name": "Authorization", "value": "…"}]},
  {"type": "sse", "name": "events", "url": "https://mcp.example/sse", "headers": []}
]
```

stdio servers always go through; `http` and `sse` ones only when the agent's `initialize` advertises them
(`mcpCapabilities`), else the terminal ends as `error` naming the server. An unknown or malformed entry is
refused by `run` with its index and name. Env and header values reach the bridge in its environment only
(never the agent's): shed never writes or logs them, and the `session` (event and `shed get`) shows the
servers with each value as `***`. A resume needs `--mcp` again.

### Session options

`--meta <json>` (the `run` method's `meta`) is one JSON object that shed sends unchanged as `_meta` on
`session/new`, on `session/load` and on each wake from hibernation; without it the requests carry no `_meta`.
shed does not read it: which keys exist is the agent's business; see the agent's own docs. It reaches the
bridge in its environment only, never the agent's command line, environment or a label. drover's lanes
(herdsman#923) and cooee's personas are its callers. A resume needs `--meta` again.

The agent's slash commands (`available_commands_update`) are the session's `commands`
(`[name, description, input hint]`); each change emits a `session` event. Run one by sending `/name …`.

## How it works

`run --acp` starts an ordinary terminal whose program is shed's bridge (`shed acp-bridge`, hidden). The
bridge speaks ACP through the [`agent-client-protocol`](https://crates.io/crates/agent-client-protocol) SDK on a
tokio runtime of its own. It runs the adapter with piped stdio, sends `initialize` (no `fs` or `terminal` client
capabilities) and `session/new` (the terminal's cwd and MCP servers) or `session/load`, and then:

- reads its PTY in raw mode: `send`, `approve` and `cancel` write one framed line (`RS`, JSON, newline); a line
  typed in `attach` is a prompt; `Esc` or `Ctrl+C` alone cancels;
- reports its events with the `emit` method and its states with `state`, as notifications over the server
  socket, written in batches by a task of their own;
- appends every event to `acp/<name>.jsonl` beside the state file, which `read` renders;
- redraws its screen (what `attach` and `shed watch` show) from those events on each change: the prompt bold
  after `›`, each tool call as one dim line (`…` running, `✓` done, `✗` failed, ACP's `kind`, the title's first
  line), approvals in yellow, the plan as a checklist and the reply's markdown lightly styled, wrapped to the
  terminal and without colour under `NO_COLOR`; only ACP's own fields are read, never an agent's wording;
- answers requests it does not offer (file system, terminals) with "method not found".

So kill, process-tree cleanup, resources and `wait` work as for any terminal. Agent text is shown with its
control characters replaced, so it cannot drive the reader's terminal. See
[decisions/0003-acp-bridge.md](decisions/0003-acp-bridge.md).

## ACP v2

The v2 draft (2026-07-20) drops JSON-RPC batches, the client file system and terminal methods, and session
modes. shed sends no batches and offers no file system or terminal methods (it answers them "method not
found"), so only modes touch it: the `mode` event (`current_mode_update`) would no longer come. shed asks for
v1 in `initialize`; an agent that answers with another version is refused and the terminal ends as `error`
with the note `acp: agent refused: … required ACP protocol version 1 but peer negotiated 2 …`.

## Accounts

shed passes its environment (the server's, `PATH` included) through to the agent and never handles provider credentials. Each agent logs in
its own way (its own login command, config file or API key variable); log in once outside shed first.

**An expired login** ends the turn at once: the state is `error` with the note `auth: <the agent's line>`
(for `claude-acp`, e.g. `auth: Failed to authenticate: OAuth session expired and could not be refreshed`),
and the `reply` event carries that text with `stop_reason` `auth_error`. To recover, check
`claude auth status` (`"loggedIn": true`), log in again (`claude auth login`, or `/login` in `claude`), then
`send` the prompt again; the session needs no restart. The known lines are listed per agent next to the
registry code (`auth_failures` in `crates/shed-server/src/agents.rs`); another agent's expired login ends
by the idle timeout instead.

**The Claude adapter** (`claude-acp`, `@agentclientprotocol/claude-agent-acp`) runs Claude Code through the
Claude Agent SDK, not the interactive `claude` CLI. Whether its usage counts against a Claude subscription plan
login, or needs an API key, depends on the adapter's login and Anthropic's current terms for the Agent SDK.
Check both before relying on a plan login, and note that an `ANTHROPIC_API_KEY` in shed's environment
reaches the agent and is likely to be used instead of the plan.

## Manual smoke (owner's machine)

Not run in CI (it needs a real login and network):

1. `shed agents add claude-acp`, with Claude Code already logged in on the plan and no `ANTHROPIC_API_KEY`
   set in the environment of `shed serve`.
2. `shed run --acp claude-acp --name smoke --cwd <a scratch repo>`.
3. `shed wait smoke --output "ACP session ready" --timeout 60`, then `shed send smoke "Say hello in one word"`.
4. `shed wait smoke --until done waiting-approval error --timeout 300`; `shed read smoke` shows the reply.
5. The plan's usage page shows the request; nothing is billed to an API key.
6. `shed kill smoke && shed rm smoke`.

## Smoke result (2026-11-05)

Run on the mini with berth `8f077cb` (shed before the rename; `--dir` in a scratch folder, `berth serve` started
under `env -i` with only `HOME`, `PATH`, `USER` and `TERM`, so no `ANTHROPIC_*` variable), adapter `claude-acp` 0.100.0
(`npx -y @agentclientprotocol/claude-agent-acp@0.100.0`, Claude Agent SDK 0.3.316, Claude Code 2.1.316 inside),
Node 24.22.2, the `claude` CLI 2.1.311 logged in with `claude.ai` on a Max plan.

- **Works on the plan login.** Steps 2–6 all exit 0: `send` "Say hello in one word", `wait --until done` reaches
  `done`, `read` shows `Hello!`, and the event log ends with `reply` (`stop_reason` `end_turn`). The session's
  transcript names `claude-sonnet-5-5`. With no API key set and the only credential the `claude.ai` login, the
  request went to the plan; the plan's usage page was not checked (the herdsman meter tracks its own accounts,
  not this login).
- **An expired login hangs the turn.** The first run, before the owner logged in again, got this as reply
  text: `Failed to authenticate: OAuth session expired and could not be refreshed` (plain `claude -p` failed the
  same way). The turn never ended: no `reply` event, so `wait --until done blocked` timed out (exit 124) after
  300 s. Check `claude auth status` (`"loggedIn": true`) before a run. Since #25 this ends the turn as
  `error` (`auth: …`) instead; see [Accounts](#accounts).

So cooee can talk to Claude through `claude-acp` on the plan login; the `claude -p` stream-json fallback is not
needed.
