# Run one

```sh
shed run --acp claude-acp --label issue=7 --cwd /home/user/project
shed send t1 "Fix the failing test"
shed wait t1 --until done waiting-approval rate-limited error
shed approve t1 allow-once        # an option id, or a kind: allow_once, reject_once, …
shed approve t1 allow-once --request 7   # the approval_request with id 7 (default: the oldest)
shed read t1                      # > prompt, [tool] title (status), the final reply
shed read t1 --events             # the same events drawn: › prompt, ✓ kind title, the reply styled
shed read t1 --events --json      # each event with its seq; page with --before <seq> --lines <n>
shed cancel t1                    # or: shed keys t1 esc
shed model t1 opus                # one of `shed get t1`'s session.models
shed run --acp claude-acp --resume <session id>   # continue a session (`agents sessions`, `shed get`'s session.id)
shed run --acp claude-acp --mcp servers.json      # with MCP servers (mcp.md)
shed run --acp claude-acp --meta '{"k": 1}'       # with session options (session-options.md)
shed send t1 "/review src/"                       # a slash command is an ordinary prompt
```

`--policy allow|deny|ask` (default `ask`) answers permission requests: `allow` and `deny` pick the first
allow or reject option at once; `ask` reports `waiting-approval` (note: the tool) until `shed approve`. With a command
after `--`, `run --acp <id> -- <cmd…>` runs that command as the adapter instead of the installed one.

The state is `working` from `send` until the turn ends, and `done` (note: the stop reason, e.g. `end_turn` or
`cancelled`) after; an agent error is `error`; [Lane states](lane-states.md) has them all. A turn also
ends as `error` when the agent's login failed (note `auth: <the line>`, see [Accounts](../accounts/accounts.md)) or when
it hears nothing from the agent for `$SHED_TURN_IDLE_TIMEOUT` seconds in the environment of `shed serve`
(default 1800; note `no response`). Anything the agent sends counts: every update (thought chunks, plans,
usage, tool-call progress), any request, and each line it writes to stderr; a permission request waiting for
`approve` does not time out. The default is long because a model thinking at high effort can send nothing
for minutes on a healthy turn, and the watchdog is off while a tool call is `pending` or `in_progress`; an expired login is caught at once by its reply, so the watchdog is only a
backstop for an agent that really hangs. `shed events`
streams `turn`, `text` (reply chunks), `tool_call` (each status change, with ACP's `locations` paths and its
`raw_input` up to 4 KiB), `plan`, `mode`, `approval_request` (its `id`, the `tool`, the `options` and ACP's
`toolCall` as `call`, without `content`), `approved`, `reply` (the whole reply and its `stop_reason`), `session` (its id, the current `model` and the
`models`, also kept as the terminal's `session`) and `usage` (the agent's `usage_update`: tokens `used` of the
context window's `size` and the session's `cost`, also kept as `session.usage`; `shed top` shows it as
`TOKENS`, e.g. `53k/200k`) next to the usual `started`, `state` and `exited`; see [protocol.md](../protocol.md).

ACP's `usage_update` has no cache counts, so for an agent whose provider record has a `[quota]` table shed
also reads the turn's tokens from each prompt response's `_meta` (for `claude-acp`, `_meta.quota.token_count`):
`usage.turn` and `usage.total` (the session's sum since the lane started, kept through a hibernation) each
hold `input`, `cache_read`, `cache_write` and `output`. A response without them, or with a counter that is
not a number, leaves both out. `shed top`'s `CACHE` is the last turn's cache reads as a share of all its input,
`cache_read / (input + cache_read + cache_write)`, and `-` without a count. A low share on a turn that
follows others means the prompt cache was lost: a model switch, a changed system prompt or tools, or a gap
past the cache's lifetime (minutes) before a hibernated lane woke. The first turn of a session is always low,
since it writes the cache. Codex reports cache reads but no cache writes, and has no `[quota]` record yet.

shed owns sessions, whatever the agent offers. Each session shed runs has a record `sessions/<id>.json` (a
`SessionRecord` in [protocol.md](../protocol.md): `id`, `agent`, `cwd`, `title`, the first prompt's first line,
`updated`, the last turn's end, and `turns`) and a transcript `sessions/<id>.jsonl`, beside the state file (0600
in a 0700 folder; in a file name each byte of the id outside `[A-Za-z0-9._-]` is written `%XX`). `read` finds a
terminal's transcript by its `session.id`, and `session.driver` says how the lane talks to its agent (`acp`, or
`stream`: [The stream driver](stream-driver.md)).
`shed agents sessions <id>` lists shed's own sessions of that agent, newest first, from the records alone: it
starts no agent and needs none installed; `--json` gives the records, each with its `transcript` path. It no
longer asks the agent (`session/list`): a session shed never ran is not listed.

`--resume <id>` (the `run` method's `resume`) needs an agent that advertises `loadSession`, else the terminal
ends as `error`. shed replays its own transcript of the session first: those events carry `"replay": true`,
so `read` shows the whole conversation, and then the agent loads the session (`session/load`); whatever the
agent replays during the load is dropped. A session shed never ran resumes with no earlier history shown,
gets its record then and is listed from then on. `shed rm` of the terminal deletes the session's record and
transcript at once (and still sends `session/close` when the agent offers it); otherwise the server deletes a
session `$SHED_SESSIONS_KEEP` days (default 30) after its last turn. Transcripts written by an older build as
`acp/<name>.jsonl` are renamed to `sessions/<id>.jsonl` when the server loads its state, so a lane still
running under the old bridge across a handover keeps writing the same file. The session id survives a server restart in the lost
terminal's `shed get`; a running ACP terminal itself survives one, its `read` and state up to date. `shed model` (the `acp.model` method) sets the agent's `model` config option
(`session/set_config_option`; the SDK shed uses has no `session/set_model`).
