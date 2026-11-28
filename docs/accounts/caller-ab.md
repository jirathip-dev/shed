# Caller name A/B: `sdk-ts` against `shed`

Claude Code sends its caller name (`CLAUDE_CODE_ENTRYPOINT`) with each request. The Agent SDK inside
claude-agent-acp sets `sdk-ts` only when the variable is unset (`if(!v.CLAUDE_CODE_ENTRYPOINT)
v.CLAUDE_CODE_ENTRYPOINT="sdk-ts"` in `@anthropic-ai/claude-agent-sdk` 0.101.0's `core.mjs`), so a value shed sets
reaches `claude`. `caller = true` in `<config>/providers/claude.toml` sets it to `shed` on the adapter and the stream
driver ([provider records](../acp/provider-records.md)); the shipped record keeps it off.

## Run (2026-11-25, claude-acp 0.101.0, one subscription login)

The same prompt, once per arm, on a fresh lane of the host's shed with `--policy deny` (no tools):

```sh
prompt='Without using any tools, write an explanation of TCP congestion control (slow start, congestion avoidance, fast retransmit, fast recovery, CUBIC, BBR) of about 800 words.'
TZ=UTC claude -p /usage                     # before
shed run --name ab384-$arm --acp claude-acp --policy deny --no-hibernate --cwd "$out" \
  [--env CLAUDE_CODE_ENTRYPOINT=shed]       # the shed arm only: what caller = true adds
shed wait ab384-$arm --output "ACP session ready" --timeout 120
ps eww -o command= -p <the lane's claude pid>   # the variable the agent got
shed send ab384-$arm "$prompt"
shed wait ab384-$arm --until done error --timeout 900
shed get ab384-$arm                         # session.usage: the tokens shed's quota counted
TZ=UTC claude -p /usage                     # after
shed kill ab384-$arm
```

| | `sdk-ts` | `shed` |
|---|---|---|
| `CLAUDE_CODE_ENTRYPOINT` of the lane's `claude` | `sdk-ts` | `shed` |
| turn | done, 1056 words | done, 999 words |
| error (400 "Third-party apps now draw from your extra usage") | none | none |
| input / cache read / cache write / output tokens | 2 / 11384 / 15373 / 2251 | 2 / 0 / 40230 / 2177 |
| context used (`usage.used`) | 29010 | 42409 |
| cost shed reports | $0.0853 | $0.1828 |
| 5h window before / after | no `Current session:` line | no `Current session:` line |
| week (all models) before / after | 64% / 64% (22:30:58Z, 22:34:45Z) | 64% / 64% (22:37:07Z, 22:40:07Z) |
| other `claude` agents on the login during the run | 30, all `sdk-ts` | 29 `sdk-ts` |

## Result

No lower draw shown, so the shipped record stays off (the issue's step 3 does not follow):

- The window could not show one lane: `/usage` had no 5h line on this login at the time, the week moves in whole
  percents, and about 30 other lanes drew from the same login during both runs.
- Under `shed`, Claude Code sent a larger prompt (40230 tokens written to the cache against 26757 read and written
  under `sdk-ts`) and shared no prompt cache with the `sdk-ts` lanes: every first turn on a new name pays a cache
  write. Any per-token discount has to beat that.
- No request was refused: the `shed` name did not hit the "Third-party apps" 400.

A useful repeat needs a login no other lane uses, a time when `/usage` shows the 5h line, and several turns per arm.
The weighting is undocumented and may change.

`CLAUDE_CODE_DISABLE_ARTIFACT=1`, which the Hermes plugin sets with its own name, was not set in either arm; whether it
explains part of the larger prompt under `shed` is untested.
