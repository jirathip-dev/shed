- Keychain and credentials-file drift: `account.status`, the status beat (every account, `default` too) and the
  end of `shed account login` compare the sha256 of an account's keychain item (read from shed serve's own
  session) with its `.credentials.json`, in memory. When both exist and differ the account is `drifted`: its
  line names the `shed account login` to run, `shed account ls` and `shed doctor` show it, and `auto` skips it.
  A keychain that cannot be read leaves drift `unknown`. A provider record's new `[credentials]` table holds the
  file, the keychain service and its per-folder suffix.
