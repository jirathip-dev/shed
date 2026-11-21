# MCP servers and slash commands

`--mcp <file>` (the `run` method's `mcp_servers`) is a JSON list of MCP servers in ACP's shapes, which shed
passes to the agent in `session/new` and in `session/load`:

```json
[
  {"name": "files", "command": "/usr/local/bin/mcp-files", "args": ["--root", "."],
   "env": [{"name": "TOKEN", "value": "…"}]},
  {"type": "http", "name": "web", "url": "https://mcp.example/", "headers": [{"name": "Authorization", "value": "…"}]},
  {"type": "sse", "name": "events", "url": "https://mcp.example/sse", "headers": []}
]
```

stdio servers always go through; `http` and `sse` ones only when the agent's `initialize` advertises them
(`mcpCapabilities`), else the terminal ends as `error` naming the server. An unknown or malformed entry is
refused by `run` with its index and name. Env and header values reach the bridge in its environment only
(never the agent's): shed never writes or logs them, and the `session` (event and `shed get`) shows the
servers with each value as `***`. A resume needs `--mcp` again.
