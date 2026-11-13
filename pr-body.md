Closes #94

## For humans

An agent started by shed can now be given session options by whoever starts it, such as "load only this project's settings, not mine" and "use only the MCP servers I give you".
A drover lane or a cooee persona can then keep the person's own global instructions, plugins, skills and connectors out while still using the normal login, so it no longer needs its own sign-in folder for that.
Starting an agent without these options sends exactly what it sent before.

## What changed

- `shed run --acp … --meta '<json object>'` (the `run` method's new `meta`, documented in `shed-proto` with drover (herdsman#923) and cooee named as callers). It must be a JSON object; shed does not read it.
- The server hands it to the bridge in `$SHED_ACP_META` (environment only, never the agent's command line, environment or a label: `runner::piped` now removes `SHED_ACP_MCP` and `SHED_ACP_META` from the agent).
- The bridge keeps it in the session and sends it unchanged as `_meta` on `session/new` and `session/load`, so a wake from hibernation sends it again. A caller's own `--resume` sends `--meta` again, as it already does `--mcp` (docs/acp.md).
- Without `--meta` the request is unchanged: the SDK skips a `None` `_meta`, and the fake agent now fails any `session/new` or `session/load` whose keys aren't exactly `cwd, mcpServers` (plus `sessionId` on load). Every existing ACP test passes with that check.

Decided: one JSON object (`--meta '{…}'`) rather than a repeated `--meta key=json`. It is exactly the `_meta` value, so there is nothing to merge.

`src/ net +20` (no budget given): the flag, the proto field, the env hand-off and two `.meta()` calls. Trimmed from +35 for the cap: the CLI parses `--meta` with clap and serde directly, the registry builds the env list in one line, and the bridge reads `$SHED_ACP_META` the same way it reads `$SHED_ACP_MCP` (the server wrote it, so it is trusted). Cap: 7815 / 10000 after #106 raised `line_cap` (owner decision q1557).

## Evidence (head 70474e45b91f2b74f5300cccf48ac12e92a11993)

Update after #106: merged `origin/staging` b5a69b2 (clean; only `ci/cap.toml` and its history note).

- `just fmt clippy banned doc cap deps hygiene neutral comments self-hosted arch lessons` (PATH plus `/usr/sbin:/sbin`): each exit 0; `just cap` prints `7815 / cap 10000`.
- `just test`: exit 101 on only `reliability::exited_sessions_release_their_fds` and `reliability::serve_raises_a_lowered_soft_limit_so_many_sessions_spawn`, both `pty: Device not configured` (the host had 431 `/dev/ttys*` in use by other lanes, near macOS's 511). `cargo test -p shed --test reliability -- --test-threads=1`: exit 101 on the same two for the same reason. No process from this worktree held ptys. This PR does not touch pty code; CI's runners decide.

### Earlier, at e53572e

Fix round 3: merged `origin/staging` 719738b (conflicts in `runner.rs`, `shed-proto/src/lib.rs`, `registry.rs`, `main.rs` and `docs/protocol.md`, resolved by keeping both sides: `--size`/`rows`/`cols` from #85 and `--meta`). Added `meta: None` to #90's new watchdog unit test, regenerated `docs/shed.openrpc.json` (#86) with `shed schema` (only the `run` description and the new `meta` param changed), trimmed for the cap, and changed the bad-`--meta` assertion to clap's "invalid value … '--meta <JSON>'" message.

- `PATH=$PATH:/usr/sbin:/sbin just test`: exit 101 on one test only, `reliability::serve_raises_a_lowered_soft_limit_so_many_sessions_spawn` (pty exhaustion at s247 on a shared host: `pty: Device not configured`); rerun alone `cargo test -p shed --test reliability serve_raises`: exit 0. An earlier full `just test` at the same code before the trims had that test passing. `/usr/sbin` is needed because this lane's PATH lacks `sysctl`; without it `machine_stats_…` fails (uptime 0), as noted in round 1.
- `just fmt`, `just clippy`, `just banned`, `just doc`, `just deps`, `just hygiene`, `just neutral`, `just comments`, `just self-hosted`, `just arch`, `just lessons`: each exit 0.
- `rg -i 'settingSources|strictMcpConfig|claudeCode' crates docs`: exit 1 (no match).
- `just cap`: exit 1, `7815 / cap 7800` (see above).
- `just temp-left`: exit 1 on `shed-watch-*` folders dated 2026-11-10/11, which no code in this repo creates (`rg shed-watch crates scripts` finds nothing); they predate this lane.
- Not run (not installed here): `just deny`, `just actionlint`. No dependency or workflow changed in this PR.

### A real session through claude-agent-acp 0.101.0 (Notes)

Ran on 2026-11-12 with this build's `shed serve` (clean env, default login), `--policy deny`, with `--meta '{"claudeCode":{"options":{"settingSources":["project","local"],"strictMcpConfig":true}}}'` and without:

- On the wire (recorded with a `tee` in front of the adapter): `"method":"session/new","params":{"cwd":…,"mcpServers":[],"_meta":{"claudeCode":{"options":{"settingSources":["project","local"],"strictMcpConfig":true}}}}`
- Slash commands: 56 with the options, 109 without.
- Global `CLAUDE.md` in context: with the options NONE, without it `# Global preferences` (session folder outside `$HOME`).
- `mcp__` tools: with the options NONE, without them the claude.ai Docs connector (`mcp__claude_ai_Claude_Docs__batch`, `…guide`, `…update`, …).

Caveat for callers: with the session folder **under** `$HOME`, Claude Code still loaded `~/.claude/CLAUDE.md` even with `settingSources: ["project","local"]`. Its project walk up from the folder reaches `$HOME/.claude/CLAUDE.md`. This is Claude Code's behaviour, not shed's, but drover's worktrees under `~/.local/share/…` will hit it.

## Why (round 1)

Decided: `--meta` takes one JSON object, sent unchanged as `_meta` on session/new and session/load
Because: "sends them as the session's `_meta` on `session/new` (and on a load or resume), unchanged"
Rejected: a repeated `--meta key=json` flag — the value already is the `_meta` object; merging keys adds code and a second shape
Decided: the server passes it to the bridge in $SHED_ACP_META, and runner::piped removes it from the agent's environment
Because: "as long as it never goes through the agent's command line or a label"
Decided: the bridge keeps it in its Session, so a wake from hibernation (session/load) sends it again
Because: "They are kept with the terminal, so a restarted or resumed session gets the same ones."
Decided: a caller's own `--resume` sends `--meta` again, the same as `--mcp`
Because: docs/acp.md:93
Check: the fake agent fails any session/new or session/load whose keys are not exactly today's (plus `_meta` only when --meta is given)
Check: the new acp test fails with the bridge's `.meta(...)` set to None (exit 101)
Check: the PR body's real claude-agent-acp run: 56 vs 109 commands, no global CLAUDE.md and no claude.ai connector with the options
Owner: an agent started by shed can be told to load only its project's settings and the given MCP servers, keeping the normal login.

<sub>note on d3dc419 · git log --notes=why</sub>

## Why (round 2)

Decided: the meta test uses made-up keys (vendorX/opts/layers/strict/n), not any agent's option names
Because: q1542
Decided: docs/acp.md describes `--meta` without naming any agent's option keys
Because: "shed never names a session option key or an agent for this passthrough"
Rejected: keeping the claude example in docs as illustration — the amendment puts Claude only in the PR's live check
Check: rg -i 'settingSources|strictMcpConfig|claudeCode' crates docs finds nothing
Owner: the docs and tests for passing session options now stay agent-neutral; the real Claude run stays only in the PR.

<sub>note on c372197 · git log --notes=why</sub>

