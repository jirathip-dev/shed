- The stream driver (off by default): a provider record with `driver = "stream"` and a `[stream]` table runs
  its lanes on the agent's own JSON-lines mode instead of its ACP adapter, read by a line format in
  `providers/<wire>.rs` (the first, `plain`, reads shed's own values). A lane goes to it only when it can be
  served whole (the policy has flags, so never `ask`; the command is the adapter or none; every `--meta`
  option has a flag); else it stays on ACP and its first line says why. A caller's wrapper around the adapter
  is kept. shed makes the session id; a cancel or a model switch stops the agent and the next send resumes
  it; MCP servers go through a private `sessions/<id>.mcp.json` removed at the agent's first line.
