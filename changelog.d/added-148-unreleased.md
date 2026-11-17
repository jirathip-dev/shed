- `shed ls --json` and `get` show each ACP session's start-to-ready time (`session.ready_ms`) and its first
  prompt's time to the first reply chunk (`session.first_token_ms`); `shed agents --json` gives each agent's
  `medians` of both over its last `$SHED_MEDIAN_OF` (default 20) sessions.
