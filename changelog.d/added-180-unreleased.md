- Claude can run on the stream driver: `claude`'s own stream-json mode with shed's bridge, no Node adapter
  (about 120 MB less per awake lane). It is off by default; a host turns it on with `driver = "stream"` in
  its own `providers/claude.toml`. Lanes with policy `ask`, another command or an unmapped `--meta` option
  stay on ACP. What a client sees differently is in docs/acp.md, "The stream driver".
