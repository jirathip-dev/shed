# Login: record keys, status, usage and redaction

The provider facts an account needs live in its record (`crates/shed-catalog/providers/<id>.toml`, or a file of
the same name in `<config>/providers/` that overrides keys of it). The design is
[docs/architecture/accounts/](../architecture/accounts/README.md); this page is what shed has today.

## Record keys

| key | what it says |
|---|---|
| `login`, `logout`, `status` | the agent's own commands to sign in, sign out and print who is signed in |
| `config_dir_var` | the variable that points the agent at its config folder, where its login lives |
| `setup_files` | the entries of that folder that are a setup (instructions, settings, hooks, skills), not a login |
| `session_files` | where a session's transcript lives in the folder; `{session}` is its id |
| `scrub` | variables, besides `[cache] api_vars`, a run on an account never inherits |
| `[status_read]` | how to read `status`: its `format` (`json`), the `signed_in` key whose `true` means signed in, the `who` keys (the first one set wins) and the `how` key (how it signed in) |
| `[usage]` | the usage probe: its `command`, `env` and `unset`, how long a reading is `fresh` and how long a limit is `hold`, and one `[[usage.windows]]` per window: its `id`, the `line` it starts with, the `used` marker after the percentage and the `resets` marker before the reset time |
| `[token]` | the setup-token command, the token's `prefix` and how many `days` it is good for |
| `[credentials]` | the drift check: the credentials `file` in the config folder, the `default_dir` under the home folder, the keychain `service` for `default` and the `folder_hash` digits of the folder path's sha256 a named folder's service ends in, the `keychain` command that prints an item (`{service}`) and the exit status it gives when the item is `missing`. Without it, no drift check |
| `[remote_login]` | the login finished on another device: its `command` (the record's `login` when unset), the text the sign-in `link` starts with, the `code` text on the line above a one-time code the link's page asks for, and the `answer`: `paste` (typed into the login), `callback` (a localhost address, fetched on the host) or `none` (the login finishes by itself). See [Remote login](#remote-login) |
| `[proxy]` | the upstream, the allowed `paths`, the variables a run gets (`base_url_var`, `key_var`), the `auth` header and headers to `append` |

Every key is optional. A record without `[status_read]`, or whose status output is not a JSON object, reads its status
by exit code: 0 is signed in, and the output (stdout, else stderr) on one line is the status. A record without `[usage]` has no usage reading.

When both the keychain item and the credentials file exist and their sha256 differ, the account is `drifted`, its line
names the `shed account login` to run, and `auto` skips it. One missing is no drift; a keychain that cannot be read
(locked, or no GUI session) leaves drift `unknown`.

## Reading the status and the usage

`Provider::read_status` takes the status command's exit, stdout and stderr and gives whether the account is
signed in, the line to show and who it is. `Provider::read_usage` takes the probe's stdout and gives each
window's percentage used and reset time, or nothing when no window line is found.

Reset times are read by `when`, always as UTC (the probe runs with `TZ=UTC`), in four forms:

- unix seconds: `1763300000`;
- a time of day, the next one: `3pm`, `3:30pm`;
- a date and time, the nearest one: `Nov 16 at 2am`;
- a delay: `in 2h 5m`.

A trailing `(UTC)` is allowed; another zone, or any other form, gives no reset time.

## Remote login

`shed account login <provider> <name> --remote` (`account.login` with `remote`) signs in without being at the host:

1. shed runs the record's `remote_login.command` (else `login`) with `open` and `xdg-open` first on `PATH` and
   `BROWSER` pointed at the same stand-in, which prints the address instead of opening a browser, on a screen
   1000 columns wide so a link never wraps;
2. it waits up to 30 s for a word starting with `link` and hands it back unredacted, with the first word on the line
   under `code` when the record has one; the login terminal stays redacted;
3. the owner opens the link on any device and sends the answer back with `account.answer` (the CLI asks for it on
   stdin): `paste` types it into the waiting login, `callback` fetches it on the host (only a `http://localhost:` or
   `http://127.0.0.1:` address), `none` refuses it, since the login finishes once the page is done.

A new agent needs only a record with this table; for example:

```toml
[remote_login]
command = "agent login --device"
link = "https://example.com/device"
code = "Enter this code"
answer = "none"
```

## Redaction

`redact` (in shed-core) replaces with `[redacted]`:

- every run of 20 or more token characters (letters, digits and `-_.+=~`) that holds both a letter and a digit;
- the value of a link's `token`, `secret`, `password` or `key` parameter.

A terminal whose `TerminalInfo` has `redacted` set (a login's) shows its text only through `redact`: `read`,
`screen.watch`, `wait`'s `output` and its events all pass through it, and `attach` is refused with `READONLY`,
since an attached client would see the raw screen.
