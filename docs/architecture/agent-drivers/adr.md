# ADR: shed owns sessions, history and resume; agents plug in as drivers (#172)

## Summary

Decision: the bridge gets one driver interface. Everything that is not an agent's wire format moves into a lane core shed owns (history, the session list, resume, cancel, model, policy, usage, limits, hibernation, states). ACP becomes one driver; a second one runs an agent's own JSON-lines mode, with Claude as its first provider (a record plus one 130-line file). It is off until a host writes one line in its own `providers/claude.toml`. Anthropic's terms allow it and the three checks pass (below).
Slices: (1) split `acp.rs` into the lane core and the ACP driver, no behaviour change, +90; (2) shed's session index, history and resume for every driver, +95; (3) the stream driver with the test fake, +330; (4) the Claude provider, measured and smoke-tested live, +135. Total about +650 lines, so one cap raise from 12,100 to 12,850 before slice 1.
Open questions: (1) drover and cooee pass shed their own adapter command, so "no drover or cooee change" needs the driver to keep their wrapper and swap only the adapter's part for `claude`: accept (recommended), or change both callers instead? (2) Policy `allow` becomes Claude's bypass mode, which drover's lane settings already ask for, with the lane's hook as the guard: accept (recommended), or auto mode with every prompt refused? (3) shed keeps a session's record and history 30 days after its last turn, and `shed rm` deletes them at once: accept (recommended)?

**Status:** proposed. Supersedes [0006](../../decisions/0006-claude-direct.md)'s "stay on the adapter".
Usage, shapes, boundaries, the statechart, the event map and the slices: [README.md](README.md).

## The question

An awake Claude lane is shed's bridge, the Node adapter and `claude`: 424 MB, of which 114 MB is an adapter
that only translates (0006). 0006 kept it because a direct bridge would lose a resumed session's history
and `agents sessions`, and would need the SDK's undocumented control protocol for approvals, cancel and
model switching. So: can shed talk to `claude`'s JSON-lines mode with documented flags only, lose nothing a
client uses, and do it in a shape where the next agent is one record and one small file, not a second
bridge?

What changed since 0006: shed already writes each lane's history itself (`acp/<name>.jsonl`); no client
passes `resume` or calls `agents sessions` (drover, cooee, 2026-11-19); `claude` 2.1.327 documents every
flag a lane without interactive approvals needs; and the owner decided that cancel and a model switch may
stop the process and resume, as a hibernation wake does.

## Options

| | option | verdict |
|---|---|---|
| A | Stay on the adapter (0006) | 114 MB per awake lane for a translator, and the Agent SDK inside a third-party adapter on a plan login is the weaker reading of the terms (below) |
| B | A one-off Claude bridge beside `acp.rs` | 0006's +500 to 800 lines for one agent, with the transcript, states, watchdog and hibernation written twice. Codex would be a third copy |
| C | **One lane core, drivers under it: ACP, and a stream driver whose providers are data plus a line parser** | chosen |
| D | Speak the SDK's control protocol for approvals, cancel and model | undocumented; the owner ruled it out |

Inside C:

- **Who owns history and resume.** The agent (ACP's `session/load` replay, today) or shed. Shed: it has
  the transcript already, a wake already ignores the agent's replay, and it makes every driver equal. The
  price: a session shed never ran resumes with no earlier history shown.
- **Cancel.** The documented choices are SIGINT ("ends the turn", then the process exits 0) and SIGTERM
  ("leaves the turn unfinished"). Both were run and both resume to a usable session. SIGTERM is what
  shed's `kill_tree` already sends for hibernation, so cancel is "end the turn in shed, then the
  hibernation path": no new stop code. SIGINT stays the fallback if an unfinished turn ever confuses a
  resumed model.
- **The session id.** Read it from the agent's first line, or mint it. Claude prints nothing before the
  first prompt, so shed mints a UUID and passes `--session-id`: the lane is ready at once and shed's id is
  Claude's id.
- **Whose turn a `result` ends.** A resumed process can print a `result` before it reads the prompt (seen:
  a background command of the stopped process being tidied up). `--replay-user-messages` echoes each
  prompt as it is taken, so a turn's end only counts after its echo.
- **The caller's own adapter command** (open question 1). drover runs
  `run --acp claude-acp -- sh -c '<exports, settings copy> && exec "$@"' sh npx -y …claude-agent-acp@…`
  and cooee `-- env K=V … <the installed adapter>`. Ignoring the command loses their environment;
  honouring it keeps the adapter. Chosen: the record names the adapter (`adapter`, `launchers`); when a
  command's last argument is the adapter, it and its launcher words are replaced by the record's
  `command` and the wrapper before them is kept. Any other command stays on ACP.
- **`--meta`.** It is the adapter's `_meta` today. The record maps each option path to a flag
  (`claudeCode.options.settingSources` to `--setting-sources`); an option with no flag keeps the lane on
  ACP, because dropping `settingSources` silently would load the owner's own setup into a lane.
- **The test fake.** A second parser written only for tests, or one that is useful: the `plain` wire reads
  shed's own `Heard` values as JSON lines. It is 20 lines, the fake agent speaks it, and any agent that
  can print those lines needs only a record.

## Decision

1. **One lane core** (`lane`), no protocol in it: inputs, turns, states, the watchdog, the process tree,
   limits and logins by the record's patterns, token sums, hibernation, the transcript, the preview, the
   writer, the session index.
2. **A driver is one awake agent process and its protocol.** It turns what the agent says into `Heard`
   values and takes `prompt`, `cancel`, `model` and `approve`. `cancel` and `model` answer "told" (ACP) or
   "stop me" (stream); a stopped lane is started again on the next send with the resume arguments.
3. **shed owns sessions.** `sessions/<id>.json` is the index record and `sessions/<id>.jsonl` the
   transcript, for every driver. `run --resume` replays from shed's transcript (`"replay": true`) and
   tells the agent to load the session; what the agent replays is dropped. `agents sessions` lists the
   index. A session shed never ran resumes with no earlier history, and is listed from then on.
4. **The stream driver** runs the record's `command` plus per-start arguments from the record: `new` or
   `resume`, `model`, the policy's flags, `mcp`, and the `meta` map. Its line format is the provider's
   Rust file (`Wire`: write a prompt, read a line, write the MCP file).
5. **Claude's flags, all documented** (`claude --help`, cli-reference, headless docs): `-p --input-format
   stream-json --output-format stream-json --include-partial-messages --verbose --replay-user-messages`,
   `--session-id`, `--resume`, `--model`, `--mcp-config`; policy `allow` is `--permission-mode
   bypassPermissions --permission-prompts none`, `deny` is `--permission-mode default --permission-prompts
   none`. Nothing is written to the agent but user messages.
6. **A lane goes to the stream driver only when it can be served whole**: the record says
   `driver = "stream"`, the policy has flags (so `ask` never does), the caller's command is the adapter or
   absent, and every `--meta` option has a flag. Otherwise it runs on ACP as today and `shed get` shows
   `session.driver` `acp`, with the reason as the lane's first line.
7. **Off by default.** The built-in record carries the `[stream]` table and no `driver` key. One host's
   `providers/claude.toml` gets the line `driver = "stream"`.

## Consequences

- **What a client sees differently on the stream driver** is listed line by line in the README's map.
  In short: usage once per turn; slash commands as names only, from the first turn; tool titles are the
  command or the path instead of the adapter's wording; no `approval_request` or `approved` events; the
  models list is the record's; a cancel also ends background commands the turn started, and the next send
  pays a wake (0006: about 3.6 s to the first update); `ready_ms` is the process start only.
- **For every driver, ACP included:** a resumed session's history is shed's, so a session started outside
  shed shows none; `agents sessions` needs no running agent and lists shed's sessions only, in shed's own
  JSON shape; a transcript outlives its terminal (open question 3).
- **MCP values touch the disk.** `--mcp-config` takes a file or a string, and a string would show in
  `ps`. The driver writes `sessions/<id>.mcp.json` (0600, in a 0700 folder) before each start and removes
  it at the agent's first line. Today they never leave memory.
- **Bypass mode has Claude's own limits.** Its docs want a non-root user for it, managed settings can
  block it (`permissions.disableBypassPermissionsMode`), and the few calls that still prompt in it are
  refused, since nobody can answer. The lane's hook stays the guard: it ran and blocked under both
  policies (below).
- **Version drift moves from the adapter to shed.** The parser reads documented message types and ignores
  the rest, and every table that can age (flags, tools, models, meta) is in the record, which a host can
  override without a rebuild. A removed flag shows as the agent's exit line.
- **A crashed agent still ends its terminal**, as today. Resuming it on the next send is now a few lines
  in the core, and its own issue.
- **Terms.** Allowed, on my reading of the texts below; the driver changes neither the binary, the login
  nor how much runs. One sentence is the owner's to weigh for a fleet on one plan: "ordinary, individual
  usage". It applies to the adapter in the same way.
- **Later, each its own issue:** making the driver the default; Codex and others after a measurement
  (#128); `run` taking an environment instead of a whole adapter command, which would retire the
  `adapter` swap; resuming a crashed agent.
- **The cap.** Staging is at 12,098 of 12,100. The slices add about 650 (README), so the orchestrator
  files one raise to 12,850 before slice 1.

## Terms (read 2026-11-19)

<https://code.claude.com/docs/en/legal-and-compliance>, "Authentication and credential use":

> **OAuth authentication** is intended exclusively for purchasers of Claude Free, Pro, Max, Ultra, Team, and
> Enterprise subscription plans and is designed to support ordinary use of Claude Code and other native
> Anthropic applications.

> **Developers** building products or services that interact with Claude's capabilities, including those
> using the Agent SDK, should use API key authentication through Claude Console or a supported cloud
> provider. Anthropic does not permit third-party developers to offer Claude.ai login into their own
> applications, or to route requests through Free, Pro, Max, or Ultra plan credentials on behalf of their
> users.

> Nor does it prevent an end user from signing in to the unmodified Claude Code binary with their own
> Claude subscription, including where a platform hosts Claude Code […]

The same page, "Acceptable use": "Advertised usage limits for Pro, Max, and Ultra plans assume ordinary,
individual usage of Claude Code and the Agent SDK."

<https://www.anthropic.com/legal/consumer-terms> (effective Oct 8, 2025) forbids: "Except when you are
accessing our Services via an Anthropic API Key or where we otherwise explicitly permit it, to access the
Services through automated or non-human means, whether through a bot, script, or otherwise." The explicit
permission is Claude Code's own documentation of this mode: <https://code.claude.com/docs/en/headless>,
"Run Claude Code programmatically" ("To run Claude Code in non-interactive mode, pass `-p`"; "bare mode
doesn't use your subscription login", so the other modes do), and
<https://code.claude.com/docs/en/authentication>: "For CI pipelines, scripts, or other environments where
interactive browser login isn't available, generate a one-year OAuth token with `claude setup-token` […]
This token authenticates with your Claude subscription".

Reading: the driver runs the unmodified `claude` binary the host installed, signed in by the owner with
the owner's own subscription, through its documented non-interactive mode. shed never sees a credential
and offers nobody else a login. That is the case the page says it does not prevent. The adapter is the
weaker case: it is a third-party program built on the Agent SDK, which the same page points to API keys.

## Checked by running

[check.py](check.py), its `hooks`, `stop`, `interrupt` and `cache` checks each run on their own (claude
2.1.327, 2026-11-19, macOS 27.2 arm64, the plan login: `apiKeySource=none`, model `sonnet`). The output
below is the script's, with the session ids shortened. Each run starts with
`claude -p --input-format stream-json --output-format stream-json --include-partial-messages --verbose
--replay-user-messages --setting-sources project,local --strict-mcp-config`, written `BASE` below, and is
stopped as `kill_tree` does (SIGTERM, SIGHUP after 100 ms, SIGKILL at 2 s).

**A lane's hooks run under both policies' flags.** The scratch folder's `.claude/settings.local.json`
(where drover puts its hook) has `PreToolUse`, `PostToolUse` (both on Bash), `UserPromptSubmit` and
`Stop`; the hook logs each call and exits 2 on a command holding `forbidden`. The prompt asks for
`touch allowed.txt`, then `touch forbidden.txt` (deny: `touch denied.txt`).

```console
$ BASE --permission-mode bypassPermissions --permission-prompts none --model sonnet
init: permissionMode=bypassPermissions apiKeySource=none claude_code_version=2.1.327
result: subtype=success is_error=False stop_reason=end_turn num_turns=3 result='finished'
permission_denials: ['Bash']
hooks.log:
  UserPromptSubmit
  PreToolUse Bash touch allowed.txt
  PostToolUse Bash touch allowed.txt
  PreToolUse Bash touch forbidden.txt
  Stop
files: ['allowed.txt']
$ BASE --permission-mode default --permission-prompts none --model sonnet
init: permissionMode=default apiKeySource=none claude_code_version=2.1.327
result: subtype=success is_error=False stop_reason=end_turn num_turns=2 result='finished'
permission_denials: ['Bash']
hooks.log:
  UserPromptSubmit
  PreToolUse Bash touch denied.txt
  Stop
files: []
```

**A process stopped mid-turn resumes to a usable session.** Stopped 3 s into a foreground
`python3 -c 'import time; time.sleep(120)'`, and with SIGKILL alone in the middle of a streamed reply:

```console
$ BASE --permission-mode bypassPermissions --permission-prompts none --model sonnet --session-id 7e67b18c-…
the command's processes before the stop: 2
stopped after the turn's first tool call: exit 143, turn ended first: False
the command's processes left: 0
$ BASE --permission-mode bypassPermissions --permission-prompts none --model sonnet --resume 7e67b18c-…
init: session_id is the same: True
result: subtype=success is_error=False stop_reason=end_turn num_turns=1 result="python3 -c 'import time; time.sleep(120)'"
second turn after the resume: subtype=success is_error=False stop_reason=end_turn num_turns=1 result='finished'
$ BASE … --session-id 1dcb47a5-…        # "Count from 1 to 400", SIGKILL after the first text
stopped after the turn's first reply: exit -9, turn ended first: False
$ BASE … --resume 1dcb47a5-…
result: subtype=success is_error=False stop_reason=end_turn num_turns=1 result='You asked me to count from 1 to 400, one number per line, with nothing else.'
```

Two edges the driver must handle, both seen: a session stopped before its first prompt was saved (no
prompt yet, or SIGKILL at the `init` line) is unknown to `--resume` ("No conversation found with session
ID", exit 1) and starts with `--session-id` again; and SIGINT mid tool call ends the turn itself
(`result: subtype=error_during_execution stop_reason=tool_use`), ends the command, exits 0 and resumes.

**The prompt cache survives a stop and a resume.** One session; cache share is
`cache read / (input + cache read + cache write)`, as `shed top`'s `CACHE`:

| turn | input | cache read | cache write | output | cache share |
| --- | --- | --- | --- | --- | --- |
| 1 | 2 | 11,617 | 6,547 | 4 | 63% |
| 2 | 4 | 13,993 | 4,185 | 4 | 76% |
| 3, the first after the stop and `--resume` | 4 | 18,178 | 21 | 6 | 99% |
| 4 | 4 | 18,199 | 16 | 4 | 99% |
| 5, the first after a restart with `--model haiku` | 2 | 11,617 | 6,903 | 106 | 62% |

Turn 5 is the model switch: `init.model` is `claude-haiku-5-5` and the conversation is written to the
cache again, as Claude's docs say of any model switch. It costs the same under the adapter.

## `.herdsman/arch.toml`

No lines to add. The one list this design could have grown, the table of parsers by name, is collected by
the build step that already collects the records (`crates/shed-server/build.rs` globs
`src/providers/*.toml`; slice 3 makes it glob `src/providers/*.rs` too), so a new provider adds two files
and edits none. `acp.rs`, 127 lines under the size limit, becomes four files by concern. The registration
lines in `registry.rs`, `main.rs` and `shed-proto/src/lib.rs` are a handful each. (The repo's arch file is
`.herdsman/arch.toml`; there is no `.drover/arch.toml`.)
