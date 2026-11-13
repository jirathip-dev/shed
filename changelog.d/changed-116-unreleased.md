- `shed serve` saves its state file at most twice a second, compact and off the registry lock, and at once on
  shutdown; `run`, `kill` and `rm` still answer only once the change is on disk. An ACP terminal's slash
  `commands` and `models` are no longer saved (the agent resends them). Exited terminals are dropped from the
  list a day after they end (`$SHED_PRUNE_AFTER` seconds of `shed serve`); their logs stay until the next start.
- Release builds use fat LTO, one codegen unit and a stripped binary: 4.0 MB instead of 8.0 MB on macOS arm64.
- `shed events --on M` with no held link is one plain stream over `ssh`: the client-side reconnect and its
  `machine: reconnecting` / `machine: connected` lines are gone; the server's held link does the reconnecting.
- The ACP path is faster per update: the bridge reads a text chunk straight from the agent's JSON instead of
  the SDK's types, writes its notifications without a `Value` between, and the server parses each request line
  once, straight into a request, and reads params without copying them.
- The server runs on one tokio runtime: connections, streams and watchers are tasks instead of threads (20
  terminals and 50 `events` clients: 24 threads instead of 143). Events go to every client from one shared log,
  each client's new events written as one batch. The ACP bridge lost its two glue threads. The CLI and the
  protocol are unchanged.
- Server responses list object keys in field order instead of alphabetically (the SDK turns on serde_json's
  `preserve_order`). The JSON is the same.
- Renamed from berth to shed: the crates (`shed`, `shed-core`, `shed-proto`, `shed-server`), the binary,
  the socket (`shed.sock`), the state and config dirs, the service units (`dev.shed.serve`,
  `shed.service`) and the docs. Terminals get `$SHED`. The `hello`
  carries `product: "shed"`, so a berth client or server is refused with `SHED_VERSION` instead of
  half-working. OSC 7501 is unchanged.
