- ACP usage carries the turn's and the session's input, cache-read, cache-write and output tokens (`usage.turn`
  and `usage.total` in `get` and the `usage` event) when the agent's prompt responses report them, as
  `claude-acp`'s `_meta.quota` does; `shed top` shows the last turn's cache-read share as `CACHE`.
