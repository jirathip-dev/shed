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
agent behaves the same outside shed. Setup adds only entries whose command holds `|| shed state `, never
twice, and leaves every other key and hook where it was; `--remove` takes out exactly those entries (and an
event list or `hooks` object it leaves empty), so setup then remove gives the file back byte for byte. A
file that ends up an empty object is deleted.

## The files each agent's setup touches

| Agent | Installed when | File | Hooks (event → state) |
| --- | --- | --- | --- |
| Claude Code | `~/.claude/` exists | `~/.claude/settings.json` (the `hooks` key) | `UserPromptSubmit` → working, `PostToolUse` → working, `PermissionRequest` → blocked, `Stop` → done, `StopFailure` → error |

Each agent's table lives in its module, `crates/shed-server/src/providers/<name>.rs`; an agent without one
is not touched.
