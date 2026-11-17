- ACP permission requests by id: `approval_request` carries its `id` (the agent's JSON-RPC request id) and ACP's
  `toolCall` as `call` (without `content`); `shed approve <t> <option> --request <id>` (the bridge's
  `{"approve": …, "request": …}` line) answers that request, and without it the oldest as before. `tool_call`
  carries ACP's `locations` (paths) and `raw_input` (left out over 4 KiB). All additive.
