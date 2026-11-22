# Lane states

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
lane. A lane on `account = "auto"` moves to the next account instead ([choice.md](../accounts/choice.md)). An agent's limit pattern is data, in its [provider record](provider-records.md):

```toml
limit = ["hit your limit", "usage limit reached"] # a reply or error line holding one of these
resets = ["resets "]                              # the reset time follows one of these on that line
```

`claude-acp` (`You've hit your limit · resets 3pm` → `3pm`) and `codex-acp` (`You've hit your usage limit. …
try again at 3:05 PM.` → `3:05 PM`) have entries. An agent without one never shows `rate-limited`; its limit
text stays its reply (`done`) or error (`error`).
