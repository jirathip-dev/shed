- `install.find`, `install.ls`, `install.stage`, `install.swap` and `install.back` move a release of shed's own
  binary or of the tool the client named at `hello` (`tool`), on the main host, and refuse any other binary with
  `NOT_OWN_BINARY`; `shed install ls` shows shed's live, staged and previous builds. Self-update runs on the same
  code, and a deferred pass now keeps the checked `<binary>.new` for the next one.
