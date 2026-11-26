wire: account.login
params: AccountLogin
result: Login

The record's `login` in a redacted terminal `login-<provider>-<name>` with the account's folder set; a second call while it runs returns the same terminal.
shed kills it after 600 s; when it ends shed runs `account.status`.
With `token`, the record's `token.command` runs instead, and shed saves the setup token it prints, read off the unredacted screen, to the account's 0600 token file.
With `remote`, the record's `remote_login.command` (else `login`) runs with `open`, `xdg-open` and `BROWSER` shadowed so no browser opens on the host, and the call waits up to 30 s for the sign-in link: the result carries it and the one-time code unredacted, and `answer` says how `account.answer` finishes it; the terminal stays redacted.
