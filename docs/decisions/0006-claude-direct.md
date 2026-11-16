# 0006: Claude stays on the ACP adapter; drop `npx`, no direct bridge

**Status:** accepted (#133). Measurement only; no runtime change.

## Context

An awake `claude-acp` lane is shed's bridge, `npx` (`npm exec`), the `claude-agent-acp` adapter (Node) and the
`claude` process the adapter's Agent SDK starts and talks to in stream-json. The owner's `ps` of ten awake lanes
(2026-11-16) showed each adapter at about 110 MB beside a 273–414 MB `claude`. The question: would a bridge that
speaks `claude`'s stream-json itself save enough to be worth a second, agent-specific protocol in shed?

Threshold, decided up front: build a direct bridge only if it saves at least 100 MB per awake session **and**
loses no state or event shed reports today, since every agent-specific protocol is a second code path to keep
working as that agent's CLI changes, where ACP keeps shed agent-neutral (#64). #128 asks the same for Codex; the
same rule answers it.

## Measurement

`scripts/measure-claude-direct.py --runs 3` (needs node, npm, npx and a `claude` login; two tiny turns per run
per mode on that login). It runs one session three ways: the adapter through `npx -y`, the same adapter installed
once with `npm install` and run by `node`, and the SDK's own `claude` binary in `-p --input-format stream-json
--output-format stream-json --include-partial-messages` (the same binary the adapter runs, so only the adapter
differs). RSS is each process's resident set from `ps`, summed over the agent's tree 2 s after the first turn
ends; shed's bridge (about 8 MB, the same in every mode) is not in it. "Ready" is the `session/new` reply (ACP) or
the `init` message (stream-json, which sends it only once the first prompt arrives). "First update" is the first
text, thought, tool call or plan of the first turn, from process start. "Wake" is the agent killed, started again
on the same session (`session/load`, `--resume`) and timed from that start to the next turn's first update.

Machine: Mac mini, macOS 27.2 arm64, 12 CPUs; Node 24.22.2; adapter `@agentclientprotocol/claude-agent-acp`
0.107.0 (Claude Agent SDK 0.3.323, Claude Code 2.1.323); 2026-11-16, medians of 3:

| mode | processes (RSS KiB, last run) | awake KiB | ready | first update | wake |
|---|---|---|---|---|---|
| `npx` | `npm exec` 115,024 · adapter `node` 131,776 · `claude` 328,000 | 574,384 | 2,339 ms | 5,412 ms | 4,049 ms |
| `node` | adapter `node` 143,184 · `claude` 326,064 | 424,368 | 1,438 ms | 4,243 ms | 4,256 ms |
| stream-json | `claude` 311,600 | 310,144 | 584 ms | 2,616 ms | 3,573 ms |

Memory was steady across runs (`npx` 537–575 MB, `node` 424–469 MB, stream-json 307–312 MB). Times were not:
the first update waits on the API, and earlier runs of the same script put `node`'s wake from 4.3 s to 7.8 s, so
only the ready time (no network) separates the modes reliably. Per awake session:

- Dropping `npx` saves its `npm exec` parent: 112–115 MB fresh (the owner's long-lived lanes showed it under
  50 MB, likely because macOS had compressed its idle pages), and about 0.9 s of start.
- A direct bridge saves the adapter on top: 114 MB (424 → 310 MB) over `node`, 264 MB over `npx`.

The 7 processes `docs/acp.md` counted for adapter 0.100.0 are not reproduced: at 0.107.0 the tree is the 3 above
plus the bridge, with no MCP servers configured. The old run's list is not recoverable.

## What the adapter gives shed, and what the stream gives directly

What shed's states and events use today (#88, #117, `crates/shed-server/src/acp.rs`), against `claude`'s stream:

| shed uses (ACP) | for | stream-json |
|---|---|---|
| `session/new` id | `shed get`, resume | direct: `init.session_id` |
| `agent_message_chunk` | `working`, `text`, the transcript | direct: `stream_event` `text_delta` |
| `tool_call` / `tool_call_update` with a title | `running` (note: the title) | events direct; the readable titles the adapter builds per tool would be redone |
| `plan` | `plan` event | mapped from the `TodoWrite` tool's input |
| `session/request_permission`, the reply | `waiting-approval`, `shed approve` | the SDK's `control_request` `can_use_tool` (`--permission-prompt-tool stdio`): the SDK's control protocol, not a documented CLI contract |
| `session/cancel` | `shed cancel`, Esc | `control_request` `interrupt` (same control protocol) |
| prompt stop reason | `done` (note), `error` | direct: `result` |
| `usage_update` (#88) | `usage` event, `shed get` | per turn only: `result.usage`, `modelUsage` |
| config option `model`, `set_config_option` | `shed model`, the models list | current model in `init`; switching and the list need control requests |
| `current_mode_update` | mode note | `init.permissionMode`; changes need a control request |
| `available_commands_update` | slash commands | `init.slash_commands`: names only, no descriptions |
| `loadSession` + `session/load` replay | `--resume` rebuilding the transcript (`"replay": true`), the hibernation wake | `--resume` continues the session but emits none of its history: **lost**, unless shed reads Claude's private session files |
| `sessionCapabilities.list` (#88) | `shed agents sessions` | none in the CLI: **lost** (the SDK reads the same private files) |
| `sessionCapabilities.close` (#88) | `shed rm` closes the session | nothing to close; no loss |
| MCP servers in `session/new` | `--mcp` | `--mcp-config` |
| reply or error text, limit pattern | `rate-limited`, auth `error` | the same text, plus a structured `rate_limit_event`: a gain |
| process tree (#117) | `running` after the turn | unchanged |

## Decision

**Stay on the adapter. Run it with `node` from a one-time install instead of `npx`.**

- A direct bridge is just over the memory bar (114 MB against 100) and fails the other half: the `"replay": true`
  history of a resumed session and `shed agents sessions` are lost unless shed parses Claude's private session
  files, and approvals and cancel would ride the SDK's undocumented control protocol. It would be one more
  protocol inside shed for one agent, the thing ACP exists to avoid. #128 gets the same answer unless Codex's
  numbers differ in kind.
- Dropping `npx` costs no state or event (the same adapter runs), saves 50–115 MB per awake lane and about 0.9 s of
  start and wake, and applies to every `npx` agent, not only Claude. It is worth having on its own.

## Consequences

- Follow-up issue: `shed agents add` installs an `npx` distribution once (`npm install --prefix
  agents/<id>/<version>`, beside the binary installs) and records `node <the package's bin>` as its command.
  Estimate: +25–40 lines in `crates/shed-server/src/agents.rs`. The repo is at 9,999 of its 10,000-line cap
  (`ci/cap.toml`), so it needs a raise first.
- Not built: the direct bridge (estimated +500–800 lines: one stream-json module plus its provider data, #64,
  #129). Revisit only if an adapter change grows it well past 110 MB, or the CLI gains documented session listing
  and history replay. Before any such build, check that Anthropic's current terms still allow a plan login in the
  unmodified `claude` binary driven this way.
- An awake lane stays about 425 MB after the `npx` change; hibernation (about 8 MB, `docs/acp.md`) remains the
  big saving for idle lanes.
