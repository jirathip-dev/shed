- `shed state` (and the agent hooks that run it) gives up after 2 s with a nonzero exit when the server
  accepts but never answers, instead of hanging; `$SHED_STATE_TIMEOUT` sets the seconds.
