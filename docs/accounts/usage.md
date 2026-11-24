# Usage and limit marks

shed reads an account's usage with the record's `[usage]` probe (its command in the account's folder, read by
the windows' markers) whenever it runs the status command: `account.status`, after a login, and on a beat.
Claude's probe is `claude -p /usage`; Codex's asks `codex app-server` for `account/rateLimits/read` and names
each window by its length (`5h`, `7d`, `30d`). Antigravity has none: its ACP server has no quota call, and
`agy -p /usage` reads agy's own login, not the account's folder.

A reading stands for `usage.fresh` seconds. Every minute shed reads again each signed-in account that has a
run and a reading older than that; `account.ls` reads a stale account again in the background, so the next
call shows it. `checked_at` is when it last read one.

An account becomes `limited` when a lane on it reports `rate-limited`. It stays limited until
`limited_until`: the reset the lane's limit line carries (read by `when`), else the reset of the fullest
window in a usage reading taken at once, else now plus the record's `usage.hold`. A later reading whose
fullest window is under 100 % ends it early. A run on a limited account is refused `ACCOUNT_LIMITED`, and
`auto` skips it ([choice.md](choice.md)).

## Balance

A provider record may have a `[balance]` table: the API's `url`, the `auth` header (`{token}` stands for the
key), the keys and indexes down to the answer's `currency` and `total` (a number or a numeric string), and
`fresh`, the seconds a reading stands. shed's built-in `deepseek` record has one:

```toml
agent = "deepseek"
token_var = "DEEPSEEK_API_KEY"
[balance]
url = "https://api.deepseek.com/user/balance"
auth = "Authorization: Bearer {token}"
currency = ["balance_infos", 0, "currency"]
total = ["balance_infos", 0, "total_balance"]
fresh = 600
```

A record with no `config_dir_var` has accounts that are only a key: `shed account add deepseek work
--token-file <file>` (0600) saves the key, makes no folder and no `default`, has no login, and the account is
`ready` while it holds a key. The probe runs with the status check, and `account.ls` reads a stale one again in
the background: one `curl` whose config, the auth header with the key in it, goes over its stdin, so the key is
in no argument, output or log. The reading is the account's `balance` (`currency`, `total`, `read_at`) and
shows in `shed account ls`'s `USAGE` column. A 401 marks the key `rejected`, and the probe sends it no more
until a new key is saved. What a low balance means (`warn_below`, an alert) is the caller's.

## Month spend

An API-key account (a record with no `config_dir_var`) adds up what its lanes use: each `usage` event of a
lane on it adds the change in the session's `total` tokens, and the change in the event's own `cost`, else
those tokens priced by the record's `[price]` table (per million tokens):

```toml
[price]
currency = "USD"
input = 0.27
cache_read = 0.07
cache_write = 0.27
output = 1.10
```

With neither, the tokens still count and the month's `cost` is `"unknown"` (also when two currencies meet).
The totals are the account's `spend` (`month` as `YYYY-MM`, `tokens`, `cost`) in `account.ls` and
`account.status`, kept in `accounts.json` across restarts and upgrades. They are this machine's only, and start
again at the first usage of a new month in the machine's local time; a past month's are not shown.
