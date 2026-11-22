# Manual smoke (owner's machine)

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
  `error` (`auth: …`) instead; see [Accounts](../../accounts/accounts.md).
