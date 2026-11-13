# 0003: ACP agents through a bridge in an ordinary terminal

**Status:** accepted (M2c, #10)

## Context

ACP agents speak JSON-RPC on stdio. shed must expose them through its existing API: `send` as a prompt,
structured events, exact `wait --until done`, approvals, cancel, and `read` as text. Terminals already give
kill with process-tree cleanup, resources, `wait`, labels and persistence; a second kind of server-owned
process would need all of that again.

## Decision

- `shed run --acp <id>` starts an ordinary PTY terminal whose program is the shed binary's hidden
  `acp-bridge` subcommand, wrapping the agent's adapter command. The ACP client itself is a library module
  (`shed_server::acp`); the binary only wires it.
- The bridge reports events through a new `emit` method (ACP event kinds only) and states through `state`, over
  the server socket. It writes a JSONL transcript (`acp/<name>.jsonl` beside the state file) that the server's
  `read` renders for terminals whose `acp` field is set.
- Inputs reach the bridge through the PTY: `send`, `approve` and `cancel` write one framed line (`RS`, JSON,
  newline); `Esc`/`Ctrl+C` alone cancels, so `shed keys <t> esc` works too.
- Installed agents live in `agents.json` beside the state file (JSON, like the state file, so no TOML
  dependency). Downloads use `curl`, checksums `shasum`/`sha256sum`, unpacking `tar`/`unzip`: no HTTP, hash
  or archive crates.
- Schema validation is structural: the test agent (`crates/shed/tests/fake_acp.py`) checks each client
  message for the fields and types ACP v1 requires and fails the run otherwise. No JSON-schema crate.
- No async runtime: one thread reads the agent, one reads the PTY, sharing one session lock. Superseded: the
  bridge runs the ACP SDK on a tokio runtime (#40), and the server is on tokio too
  ([0004](0004-tokio.md)).

## Consequences

- An ACP terminal is killed, sampled, listed and waited on like any other, and its transcript outlives a
  server restart. It cannot be `--keep` (it would come back as a shell).
- A server restart drops the bridge's socket connection; it reconnects once per call, and the session goes on.
- The bridge outlives the server (#75). The server hands the bridge a copy of its PTY's master at spawn (over a
  socket pair given as the bridge's stderr, which the bridge then points back at the PTY), so the server's exit
  neither hangs up the PTY nor, on a stop, kills the bridge. The bridge lends the master back on
  `pty-<name>.sock` beside the server's socket, with its last reported state; a starting server takes it there
  and adopts the terminal by its saved pid. A lane whose socket does not answer is marked `exited`. Chosen over
  a double-forked bridge outside the server's session: that would still need a way to give the new server the
  PTY, and fd passing does that alone, also after a crash, with no `unsafe` (`pre_exec`) and no new process.
  Output the bridge writes while no server reads stays in the PTY's buffer (the transcript has it all); its
  screen redraws are non-blocking, so a full buffer drops frames instead of stalling the session. An adopted
  bridge is not the server's child: its end is seen when the PTY closes, as exit 0.
- `shed hello` names a server of another build (another git sha) and exits 1; a bridge of an older build still
  talks to a newer server of the same protocol.
- `curl` and `tar` must be on the PATH for binary agents; `npx` or `uvx` for those distributions.
- Python 3 runs the test agent in CI.
- A bridge that cannot start its agent or finish the handshake reports state `error` with the reason.
