wire: account.login
params: AccountLogin
result: Login

The record's `login` in a redacted terminal `login-<provider>-<name>` with the account's folder set; a second call while it runs returns the same terminal.
shed kills it after 600 s; when it ends shed runs `account.status`.
With `token`, the record's `token.command` runs instead, and shed saves the setup token it prints, read off the unredacted screen, to the account's 0600 token file.
