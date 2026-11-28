wire: account.answer
params: AccountAnswer

The answer to the account's running remote login (`account.login` with `remote`), delivered the way its record's `remote_login.answer` says: `paste` types it into the login, `callback` fetches it on the host (only a `http://localhost:` or `http://127.0.0.1:` address); `none` takes no answer and is refused.
