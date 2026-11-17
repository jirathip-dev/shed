- `shed run --acp … --warm` keeps one ready spare session per recipe (agent, command, folder, policy, MCP
  servers, meta, size): the next `--warm` run of that recipe takes it, renamed to its name and labels, with
  `session.ready_ms` 0, and a new spare starts. Spares are listed as `.warm-N`; `$SHED_WARM_MAX` (default 2)
  caps them and `$SHED_WARM_IDLE` (default 600 s) kills one nobody took.
