- `shed account login <provider> <name> --remote` (`account.login` with `remote`) signs in from any device: no browser
  opens on the host, the sign-in link (and a one-time code) comes back unredacted, and the answer goes back with
  `account.answer` (the CLI asks for it): pasted into the login for claude, fetched on the host as the localhost
  callback for antigravity-acp, none for codex's device-code login. Each provider's way is its record's
  `[remote_login]` (docs/accounts/login.md).
