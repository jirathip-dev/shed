# The account store and its lifecycle

shed keeps the accounts on the main host, beside its state file, in a folder of mode 0700:

```
accounts/
  accounts.json              what shed last read of each account: status line, who, when, usage (0600); no credential
  <provider>/<name>/         an account folder: the CLI's own config folder, made empty by shed (0700)
  <provider>/<name>          or a link to an adopted folder, which stays where it is
  <provider>/<name>.token    a setup token (0600); shed refuses to read one at any wider mode
```

`<provider>` is a record's `agent` name and needs a `config_dir_var`; `<name>` is `[a-z0-9-]+`. The name
`default` is reserved: it is the CLI's own default folder (no variable set) and has no folder here.

| method | what it does |
|---|---|
| `account.add` | makes the empty folder; `folder` adopts an existing one by link; `token_file` (a 0600 file on this host) is copied in; the same call twice is one account |
| `account.ls` | per provider `default` first, then by name: what shed last read, the token's dates (never the token) and the `setup_files` the folder holds |
| `account.login` | the record's `login` in the redacted terminal `login-<provider>-<name>`, the folder set; a second call while it runs returns it; shed kills it after 600 s and runs `account.status` when it ends |
| `account.status` | the record's `status` and `[usage]` probe now, with the folder set and none of the record's `scrub` or `api_vars` inherited |
| `account.rm` | the record's `logout`, then the folder (an adopted one: only its link), the token and the reading; refused for `default` and while the login runs |

An account is `ready` when its status command last said signed in, or when it has a token; else `signed-out`.

A folder holding one of the record's `setup_files` is a setup, not a login: `account.add --folder` refuses it
with `ACCOUNT_SETUP`, and `account.ls` lists what a folder shed made has gained since.

The token is a `Token` in `shed-accounts`: it has no `Debug`, `Display`, `Serialize` or `Clone`, and no method
takes or returns one. Only a run's environment and the token proxy may read it (`.expose()` is banned elsewhere).

`shed doctor` adds a row for each named account that is signed out or holds a setup, and for each token file:
its age, its days left, and a `chmod 600` when it is open to others.
