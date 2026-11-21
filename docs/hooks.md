# State hooks for PTY agents

A program in a shed terminal reports its state with `shed state`, the socket or OSC 7501. An agent that does
none of these falls back to the silence timer. `shed setup` wires an installed agent's own hooks to
`shed state` instead, for agents run without herdsman (herdsman installs its lanes' hooks itself).

```sh
shed setup              # every installed agent shed has a hook table for
shed setup claude       # one agent; an unknown or uninstalled one is an error
shed setup --dry-run    # print each file as it would become; write nothing
shed setup --remove     # take shed's entries out again
shed doctor             # one `hooks` line per installed agent: `ok`, or `--` with the fix
```

Each hook runs `[ -z "$SHED" ] || shed state <state>`: a no-op that exits 0 when `$SHED` is unset, so the
agent behaves the same outside shed. Setup edits the file as text: it inserts shed's entries, as compact JSON
and never twice, right after the bracket that opens their list, and leaves every other byte where it
was. `--remove` deletes exactly the text setup inserted, so setup then remove gives the file back byte for
byte; entries the agent has since rewritten in its own layout are not recognised and stay. A file shed
created is deleted again while it is still exactly as shed wrote it.

## Screen rules

Some screens fire no hook: Claude Code's folder-trust question, for one. A terminal labelled `agent=<agent>`
(`shed run --label agent=claude`) also checks its provider record's `[[rules]]`, at most every 250 ms and
only after the screen changed; other terminals check nothing. A rule matches while one of its `any` texts
shows in the screen's last `region` non-blank lines; the highest `priority` wins. The agent's own report has
priority 0: a rule above it wins while it matches and stops when the screen moves on, one at or below it
applies only when there is no report. `get` shows where the state came from in `reported.from`:
`reported`, `rule:<id>` or `silence`. Each shipped rule names the agent `version` its screen was captured
from, kept in `crates/shed-server/tests/screens/<agent>-<id>.txt`.

## The files each agent's setup touches

| Agent | Installed when | File | Hooks (event → state) |
| --- | --- | --- | --- |
| Claude Code | `~/.claude/` exists | `~/.claude/settings.json` (the `hooks` key) | `UserPromptSubmit` → working, `PostToolUse` → working, `PermissionRequest` → blocked, `Stop` → done, `StopFailure` → error |

Each agent's table is the `[hooks]` of its provider record, `crates/shed-catalog/providers/<name>.toml` or
a user file overriding it (docs/acp/provider-records.md); an agent without one
is not touched.
