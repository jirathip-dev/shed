- One server per state file: `shed serve` holds a lock (`state.lock` beside the state file) for its life, so a
  second server is refused even when the first one's socket file is gone, instead of restoring the kept
  terminals twice and taking the socket. A killed server's lock goes with it. Two `run --autostart` at once
  share the one server that wins.
- `shed rm` also removes an ACP terminal's transcript, and `shed serve` removes the logs and transcripts of
  terminals its state file no longer names.
- `shed machine add|rm` writes the config through a temp file of its own, so writers running at once no longer
  fail on a shared one.
