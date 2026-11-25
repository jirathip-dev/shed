# 0005: Codex goes to the stream driver on `codex exec --json`; no app-server bridge

**Status:** proposed (#128). Measurement only; no runtime change. The build is its own issue (below).

## Context

shed reaches Codex through `codex-acp`, one process tree per session (0003). Codex also ships `codex
app-server`, one JSON-RPC process hosting many threads, and `codex exec --json`, a JSON-lines mode that runs
one turn per process and resumes with `codex exec resume <id>`. #128 asked whether a second bridge kind that
speaks app-server (one server per account, sessions as threads) is worth building. Its amendment (q2141)
added the third option: Codex as a provider on shed's stream driver ([agent drivers
ADR](../architecture/agent-drivers/adr.md), #172), measured the way 0006 measured Claude.

Threshold, from the amendment: build only if it saves **at least 100 MB per awake lane**. The issue's first
bar (10 sessions save 500 MB, or wake more than 1 s faster) is reported too.

One finding changes the question: `codex-acp` 2.x is itself a Node front over `codex app-server`. Each ACP
session today is Node plus its own app-server, so "app-server or the adapter" is really "one shared
app-server, or one per session behind Node".

## Measurement

`scripts/measure-codex.py --runs 3` (needs node, npm, npx, `codex` signed in, and `shed` for the lane
mode; about 50 tiny turns per run on that login). Five modes:

- **acp (npx)**: `npx -y @agentclientprotocol/codex-acp@2.12.0`, as the installed record runs it today;
- **acp (node)**: the same adapter installed once and run by `node`, as #170 runs npx agents;
- **app-server**: one `codex app-server`, every session a thread on it;
- **exec**: `codex exec --json`, one process per turn, `codex exec resume <id>` for the next;
- **lane**: real `shed` lanes on codex-acp, each given one turn and then hibernated.

Memory is each process's macOS physical footprint (`footprint -p`), summed over the agent's tree 2 s after
the last turn; shed's bridge is not in it except in the lane row. Footprint, not RSS: macOS compresses idle
Codex pages, so RSS of the same ten npx sessions swung from 1.1 to 1.9 GB between runs while footprint stayed within
5%. Both are in the script's output. "Ready" is process start to the session id (`session/new` reply,
`thread/start` reply, `thread.started` line). "First update" is the prompt sent to the first text, thought
or tool item. "Wake" is the sessions stopped and one started again on the same session, from that start to
the next turn's first update; app-server's warm wake is `thread/unsubscribe` then `thread/resume` on the
running server.

Machine: Mac mini, macOS 27.2 arm64, 12 CPUs; Node 24.22.2; `codex-cli` 0.171.0 (also the one the adapter
bundles); ChatGPT login; 2026-11-25, medians of 3:

| mode | 1 session | 5 sessions | 10 sessions | processes (10) | ready | first update | wake |
|---|---|---|---|---|---|---|---|
| acp (npx) | 175 MB | 857 MB | 1,659 MB | 30 | 16.7 s | 6.6 s | 16.0 s |
| acp (node) | 124 MB | 566 MB | 1,111 MB | 20 | 5.6 s | 5.5 s | 8.8 s |
| app-server | 50 MB | 79 MB | 108 MB | 1 | 0.77 s | 5.1 s | 6.0 s cold, 1.3 s warm |
| exec | 0 between turns; 48 MB during one | 0 | 0 | 0 | 0.36 s | 7.3 s | 6.5 s |
| lane, hibernated | 3.5 MB | 17 MB | 35 MB | 10 bridges | 12.1 s | | 11.7 s |

(MB here is MiB from the script's KiB.) Per process, last run: `npm exec` 45–47 MB, the adapter's `node`
51 MB, each adapter's own `codex app-server` 57–63 MB; a shared app-server grows about 6 MB per thread
from 50 MB; an `exec` turn's `codex` peaks at 49 MB (one run of three sampled no footprint before the process
exited), plus the user's shell snapshot it runs.

Times wait on the API: first update ranged 4.0–9.5 s within every mode, so only ready and wake separate the
modes. codex-acp's own start is slow (5.6 s by `node`, 11.7–16.9 s by `npx`, against 0.8 s for the same
app-server started directly), and that start is on every wake from hibernation.

Per awake lane against acp (node), the baseline once #170 drops `npx`:

- **app-server**: 100 MB a lane at 10 lanes (1,111 → 108 MB), 74 MB at one lane (124 → 50 MB). The saving
  reaches 100 MB only once about ten lanes share one account's server.
- **exec**: a lane between turns is its hibernated bridge, 3.5 MB, so it saves 108–120 MB a lane at any
  count, and during a turn its process is 48 MB against 124 MB.

Both pass the issue's first bar (1,003 and 1,111 MB saved at 10 sessions; app-server's warm wake 7.5 s
faster, exec's wake 2.3 s faster).

## What each gives shed, and what it lacks

What shed's states and events use (`crates/shed-server/src/lane.rs`, the drivers), against Codex's two
native modes. The adapter forwards nearly all of app-server: usage with cache reads and writes, fork,
resume, session listing, plans, approvals and interrupt are all mapped in `codex-acp` 2.12.0; it drops
the review-mode entry item and nothing a shed state needs.

| shed uses | app-server | exec --json |
|---|---|---|
| session id | `thread/start` reply | the `thread.started` line; no flag sets it, so shed reads it instead of minting one |
| text for `working`, `text`, the transcript | `item/agentMessage/delta` | `item.completed` only: whole messages, no streaming |
| tool calls for `running` | `item/started`, `item/completed` | the same, as `item.started` / `item.completed` |
| `plan` | `turn/plan/updated` | the `todo_list` item (documented; these tiny turns made none) |
| approvals (`ask`) | `item/*/requestApproval` | none: `ask` stays on ACP, as for Claude (ADR decision 6) |
| policy `allow` / `deny` | `approvalPolicy`, `sandbox` | `--dangerously-bypass-approvals-and-sandbox` / `-s read-only` |
| cancel | `turn/interrupt` | stop the process: the stream driver's "stop me" already |
| model, effort | per turn | `-m`, `-c model_reasoning_effort=` per turn |
| usage | `thread/tokenUsage/updated`, with cache reads | `turn.completed.usage`, with `cached_input_tokens` |
| limits, sign-in errors | `error`, `account/rateLimits/updated` | `turn.failed` / `error` text: the record's patterns |
| MCP servers | config per thread | `-c mcp_servers.<name>…` overrides, no file flag |
| history, resume, `agents sessions` | `thread/resume`, `thread/list` | shed's own (ADR decision 3) |

## Decision

**Build Codex as a stream-driver provider on `codex exec --json`. Do not build an app-server bridge.**

- exec clears the 100 MB bar at every lane count, by 108–120 MB a lane, with no new protocol kind and no
  shared process: a crashed turn ends one lane, as today.
- app-server reaches it only with about ten lanes on one account, needs a third driver kind (JSON-RPC
  with threads, its own approvals), and one crash would end every Codex session on that account. Its one
  edge, a 1.3 s warm wake, matters little when exec's process start is 0.36 s.
- The cost: text arrives as whole messages rather than streamed, and `ask` lanes stay on ACP.

## Consequences

- Follow-up issue: the Codex provider on the stream driver. It is more than "one record plus one small
  file", because `codex exec` takes one prompt and exits: the driver gains a one-turn mode (the prompt as
  the last argument with stdin closed, a clean exit after the turn's end counted as hibernated, not
  crashed) and takes the session id from the agent's first line. Estimate: `providers/codex.rs` +110–130,
  the driver +40–60, the `[stream]` table in `providers/codex.toml` (data, not counted); about +150–190
  `src/` lines against `ci/cap.toml`. Lanes passing `--mcp` stay on ACP until the record maps servers to
  `-c` overrides.
- Until then, Codex lanes gain most from #170's `node` start: 1,659 → 1,111 MB for 10 awake lanes, and
  wake 16.0 → 8.8 s.
- Not built: the app-server bridge. Revisit only if exec loses a documented flag shed needs, or many lanes
  per account become the normal load.
