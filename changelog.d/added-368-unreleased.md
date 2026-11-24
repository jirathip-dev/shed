- Codex accounts report their usage windows (`5h`, `7d` or `30d`, used % and reset) like Claude's, read from
  `codex app-server`'s `account/rateLimits/read`; `auto` and the switch on a limit use them. Antigravity has no
  way to read its windows per account, so it reports none.
