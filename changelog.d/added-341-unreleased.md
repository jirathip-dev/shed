- `shed_client::fake::Server` is a scriptable test double: per-method handlers over a shared `World`, built-in
  `run`, `get`, `ls`, `kill`, `rm`, `send`, `state`, `exec`, `file.read` and `file.write`, cued agent replies,
  live `events` streams a test pushes to until it closes them, `restart` and a forgotten ring (`reset`).
- cooee: pin shed at this release to swap its own fake of shed for `shed_client::fake`.
