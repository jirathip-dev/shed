- An ACP or stream agent no longer inherits its lane's own variables (`SHED_ACP_HIBERNATE_AFTER`,
  `SHED_ACP_RESUME`, `SHED_ACP_MCP`, `SHED_ACP_META`), so a `shed serve` the agent starts, such as one in its
  tests, no longer picks up the lane's `--no-hibernate`.
