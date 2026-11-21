# How it works

`run --acp` starts an ordinary terminal whose program is shed's bridge (`shed acp-bridge`, hidden). The
bridge speaks ACP through the [`agent-client-protocol`](https://crates.io/crates/agent-client-protocol) SDK on a
tokio runtime of its own. It runs the adapter with piped stdio, sends `initialize` (no `fs` or `terminal` client
capabilities) and `session/new` (the terminal's cwd and MCP servers) or `session/load`, and then:

- reads its PTY in raw mode: `send`, `approve` (`{"approve": "<option>", "request": "<id>"}`, no `request`: the
  oldest) and `cancel` write one framed line (`RS`, JSON, newline); a line
  typed in `attach` is a prompt; `Esc` or `Ctrl+C` alone cancels;
- reports its events with the `emit` method and its states with `state`, as notifications over the server
  socket, written in batches by a task of their own;
- appends every event to its session's transcript `sessions/<id>.jsonl` beside the state file, which `read`
  renders, and rewrites the session's record when the session starts and at each turn's end;
- redraws its screen (what `attach` and `shed watch` show) from those events on each change: the prompt bold
  after `›`, each tool call as one dim line (`…` running, `✓` done, `✗` failed, ACP's `kind`, the title's first
  line), approvals in yellow, the plan as a checklist and the reply's markdown lightly styled, wrapped to the
  terminal and without colour under `NO_COLOR`; only ACP's own fields are read, never an agent's wording;
- answers requests it does not offer (file system, terminals) with "method not found".

So kill, process-tree cleanup, resources and `wait` work as for any terminal. Agent text is shown with its
control characters replaced, so it cannot drive the reader's terminal. See
[decisions/0003-acp-bridge.md](../decisions/0003-acp-bridge.md).
