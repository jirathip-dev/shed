Closes #88

## For humans

ACP terminals now show how many tokens the agent's session uses, in `shed get`, as an event and in a new TOKENS column of `shed top`.
`shed agents sessions <agent>` lists an agent's past sessions, so resuming one no longer needs a remembered id.
Removing a finished ACP terminal also closes its session with the agent, when the agent supports that.
An agent that only speaks the coming v2 of the protocol is refused with an error naming both versions; docs/acp.md says which v2 removals affect shed.

## What changed

- **usage_update**: the bridge keeps it as `session.usage` (`used`, `size`, `cost`) and emits a `usage` event; the server keeps it on the terminal's session. `top` (not `ls`) gains TOKENS, e.g. `53k/200k`. The `session` event's payload is boxed, so an event is smaller than on staging (CI's `bench-fanout` had the head at 115% of base).
- **`shed agents sessions <id>`**: starts the installed agent, `initialize`, `session/list` following every `nextCursor`, stops it. Refused without `sessionCapabilities.list`. `--json` gives ACP's session objects.
- **rm closes the session**: `rm` only takes exited terminals, so the agent is gone by then; the server starts the terminal's agent once in its folder, sends `initialize` and, only with `sessionCapabilities.close`, `session/close`, then stops it (30 s limit; a failure never fails the `rm`). Hibernation still uses `session/load`.
- **v2**: the pinned SDK already rejects a negotiated version other than 1 (`required ACP protocol version 1 but peer negotiated 2`); shed surfaces that as the terminal's error. docs/acp.md: of v2's removals (batches, client fs/terminal, modes) only modes touch shed.
- The bridge's agent start/stop and handshake are shared helpers, reused by the one-shot list and close calls.
- No new dependency: `session/list`, `session/close` and `usage_update` are stable in the pinned `agent-client-protocol 5.0.0` (schema 1.15.1).

src/ net +122 (no budget in the issue): three ACP features with their CLI. Merged onto staging afc2107 the total is 8046 / cap 10000.

## Evidence (head 8569f00eedf65eb50fd8daec0fdaacab097ab882)

Update round 3: merged `origin/staging` afc2107 (#76). Conflicts: `crates/shed-server/src/lib.rs` takes staging's `kill`/`rm`/`read`/`input` helpers, and `rm` still closes the ACP session after the removal; `docs/protocol.md` and `protocol_doc.rs` take staging's `stuck` text plus this branch's `usage` clause. Then `EventKind::Session` boxed (`AcpSession.usage` no longer boxed), fixing the `bench-fanout` FAIL at 9ef332c (head/base 115%).

- `just fmt`, `clippy`, `banned`, `doc`, `cap` (8046 / 10000), `deps`, `arch`, `hygiene`, `neutral`, `comments`, `self-hosted`, `lessons`: each exit 0. `deny` and `actionlint` are not installed on this host (no dependency or workflow change); CI runs them.
- `just test`: exit 0 (second run). The first run had one failure in staging's #76 timing test `lock::killing_a_program_that_ignores_sigterm_holds_up_no_list` (`ls took 125ms`, limit 100 ms, under the full parallel suite); `cargo test -p shed --test lock` alone: exit 0. This branch does not touch kill or ls.
- `just temp-left`: exit 1 on older `shed-*` folders other lanes left in the shared TMPDIR; a listing before and after `just test` shows 0 new folders.
- `scripts/bench-fanout.sh <staging afc2107> .`: exit 0, fan-out median base 15065 us, head 15169 us (100%). ACP bench: CI's `bench-acp` passed at 9ef332c; this round changes no bridge hot path beyond the box.

## Why (round 1)

Decided: rm starts the removed terminal's agent once (initialize, then session/close only when it advertises sessionCapabilities.close, 30 s limit) and stops it
Because: "`rm` of an ACP terminal closes its session (`session/close`) when the agent supports it"
Because: crates/shed-server/src/registry.rs:666
Rejected: closing on kill — kill signals the bridge and the agent together, so no live agent is left to answer a close
Decided: the v2 refusal is the SDK's own (it rejects a negotiated version other than 1 before shed sees the reply); docs/acp.md names the error
Because: "The bridge refuses a v2-only agent with a clear message until v2 is supported."
Rejected: a shed-worded version check — dead code: with the fake agent's --v2 the SDK errored first
Decided: usage lives on the session in the bridge and the server (each session event carries it); a usage update does not rewrite the state file
Because: "`usage_update` is stored on the session and shown in `get` (`session.usage`), with a `usage` event."
Decided: the TOKENS column is in top only; ls keeps its columns
Because: "`shed top` gains a tokens column for ACP terminals."
Check: rm_closes_the_session_once_and_only_when_the_agent_can: the fake records any close it gets, so the no-capability case would show one
Check: session.usage is boxed so every event grows 8 bytes, not ~56: the shed-only bench went 111% -> 96% of base
Owner: ACP terminals now show token use, list an agent's past sessions to resume, and close the session when removed.

<sub>note on b3cd956 · git log --notes=why</sub>

## Why (round 2)

Decided: merged origin/staging (#86 OpenRPC); protocol_doc.rs imports keep both sides, AcpUsage derives JsonSchema, docs/shed.openrpc.json regenerated by shed schema
Because: crates/shed-proto/src/lib.rs:794
Rejected: none
Check: just cap is 7925/10000 at this head (staging b5a69b2, #106 raised the cap)
Owner: This branch is up to date with staging and under the raised line cap.

<sub>note on 9ef332c · git log --notes=why</sub>

