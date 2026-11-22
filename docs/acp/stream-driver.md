# The stream driver

A lane can skip the ACP adapter and run the agent's own JSON-lines mode instead: shed's bridge and `claude`,
no Node process (#180; design in [the agent drivers ADR](../architecture/agent-drivers/README.md)). The built-in
`claude-acp` record carries the `[stream]` table (`crates/shed-catalog/providers/claude.toml`) and no
`driver` key, so it is off. A host turns it on with one line in its own record, read at each `run`:

```sh
echo 'driver = "stream"' >> ~/.config/shed/providers/claude.toml   # macOS: ~/Library/Application Support/shed/providers/
shed run --acp claude-acp --policy allow --name t1 --cwd /home/user/project   # as before
shed get t1 --json | jq -c '.session | {id, driver}'                          # {"id":"…","driver":"stream"}
```

The lane goes to the driver only when it can be served whole: the record says `driver = "stream"`, the policy has
flags (`allow`: `--permission-mode bypassPermissions --permission-prompts none`; `deny`: `--permission-mode
default --permission-prompts none`; `ask` has none), the `run` command is absent or ends in the adapter
(`…claude-agent-acp…` as its last word, after `npx -y` or `node`; that part is replaced by `claude` and the
wrapper before it, drover's `sh -c '…' sh`, is kept), and every `--meta` option has a flag in the record's
`[stream.meta]` (drover's `claudeCode.options.settingSources`, `settings` and `strictMcpConfig`, cooee's
`tools` and `systemPrompt.*` have). Otherwise it runs on ACP as above, `session.driver` is `acp` and the bridge's
`--why` names the reason. `claude` runs as `claude -p --input-format stream-json --output-format stream-json
--include-partial-messages --verbose --replay-user-messages`, then `--session-id <id>` (an id shed made) or
`--resume <id>`, `--model`, the policy's flags, `--mcp-config <file>` and the meta flags; shed writes it nothing
but user messages. Policy `allow` is Claude's bypass mode: the guard is the lane's own hook (drover's
`PreToolUse` blocks a call under it, as under the adapter), and a call that would still prompt is refused.

What a client sees differently, from the ADR's map (everything else is as on ACP):

- `session.model` is empty until the first turn (Claude's `init` line names it), and `session.models` is the
  record's list; `shed model` passes any name on, and a wrong one fails the next turn.
- `session.commands` are names only (no description or hint), from the first turn.
- `session.ready_ms` is the process start only; the agent's own start is in `first_token_ms`.
- `tool_call`: `pending` when the call starts streaming, `in_progress` with its `raw_input`, `completed` or
  `failed` from its result. The title is the command, path or query (the record's `[stream.tools]`), else the
  tool's name; `locations` is that path, without lines. Calls still open when a turn is cancelled end `failed`.
- `plan` is `TodoWrite`'s `todos`: `content` and `status`, no `priority`.
- `mode` is `init`'s `permissionMode`, seen when a turn starts.
- No `approval_request` or `approved`, and never `waiting-approval`: the flags decide inside Claude. A refused
  call is a `tool_call` `failed`.
- `usage` comes once per turn, at its end: the last request's input and cache tokens of the model's
  `contextWindow`, and `total_cost_usd` as `cost`; `usage.turn` and `usage.total` as before.
- `rate-limited` also follows a `rate_limit_event` whose `status` is `rejected` (note: its `resetsAt`, unix
  seconds), besides the limit pattern.
- `shed cancel` stops the agent's process (it ends background commands the turn started too) and reports the
  turn `cancelled`; `shed model` is kept for the next start. Either way the next send starts `claude` again
  with `--resume` on the same session id and pays a wake (about 3 s to the first update). A model asked for in
  a turn waits for the turn's end.
- `--mcp` servers are written to `sessions/<id>.mcp.json` (0600) in Claude's `mcpServers` shape for the start
  and removed at the agent's first line; `http` and `sse` are always taken.
- `shed rm` deletes shed's files; Claude's own session file stays until Claude cleans it.

Measured on 2026-11-19 (`scripts/measure-claude-direct.py --runs 3`, macOS arm64): an awake lane on the driver
is 332,896 KiB (bridge and `claude`) against 457,648 KiB for the adapter run by `node` and 575,280 KiB by `npx`.
