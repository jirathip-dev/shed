- Agent facts are data: each agent's limit and login-expired lines, hook table, login, status and logout
  commands, config-folder and settings variables and effort flags are one TOML provider record, built in
  from `crates/shed-server/src/providers/<name>.toml` and overridden key by key, or added, by `<name>.toml`
  in `providers/` beside the config file, without a rebuild. `shed doctor` names a bad user file, which is
  skipped. `shed agents --json` and the new `agents` method list each record (with its `source`) beside the
  installed adapter (now under `installed`); `agents.match {id, text}` applies an agent's limit and auth
  rules to a caller's text. See docs/acp.md.
- cooee and drover: read program facts from `shed agents --json` in place of their own copies.
