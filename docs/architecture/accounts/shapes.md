# Shapes: accounts, tokens, keys, the record's new tables, the changed API types

All wire types live in `shed-proto` (after #248: `src/accounts.rs`, one area file). Times are unix seconds.
`PROTOCOL` becomes 5; nothing is dropped or retyped, every new field has a default.

## On disk, main host only

Beside the state file, in a folder of mode 0700:

```
accounts/
  accounts.json              what shed last read: status, usage, limit marks, token dates (0600); no credential
  <provider>/<name>/         an account folder: the CLI's own config folder, made empty by shed (0700)
  <provider>/<name>          or a link to an adopted folder; the account's folder is the link's target, as written
  <provider>/<name>.token    a setup token (0600); shed refuses to read it at any wider mode
proxy-keys.json              the per-run keys, each as its sha256 (0600)
```

`<provider>` is a record's `agent` name; `<name>` is `[a-z0-9-]+`. The name `default` is reserved: it is the
CLI's own default folder (no variable set) and has no folder here.

## An account

```rust
pub struct Account {
    pub provider: String,
    pub name: String,
    pub folder: Option<PathBuf>,       // none for `default`
    pub adopted: bool,                 // a link: `account.rm` removes the link, never the folder
    pub state: AccountState,
    pub who: Option<String>,           // the email or organisation the status command reports
    pub line: String,                  // the status on one line, redacted, at most 80 characters
    pub checked_at: Option<u64>,       // when the status command last ran
    pub limited_until: Option<u64>,    // set while `state` is `limited`
    pub usage: Option<Usage>,
    pub token: Option<TokenInfo>,      // never the token
    pub setup: Vec<String>,            // the record's `setup_files` found in the folder; not empty: no run
    pub runs: u32,                     // terminals on it now
    pub login: Option<String>,         // its login terminal, while one runs
}

pub enum AccountState { SignedOut, Ready, Limited }   // ready: the status command says signed in, or it has a token

pub struct Usage { pub windows: Vec<Window>, pub read_at: u64 }
pub struct Window { pub id: String, pub used_pct: u32, pub resets_at: Option<u64> }

pub struct TokenInfo { pub saved_at: u64, pub expires_at: Option<u64>, pub rejected: bool }
// expires_at: saved_at + the record's `token.days`; rejected: the upstream answered 401 to it
```

Headroom is `100 - used_pct` of the fullest window; a window past its `resets_at` reads as 0 % used.

In `shed-accounts` only, never on the wire:

```rust
pub struct Token(String);              // no Debug, Display, Serialize or Clone
impl Token { pub(crate) fn expose(&self) -> &str }

pub enum Choice { Named(String), Auto }                 // from `RunParams::account`
pub struct Injected { pub set: BTreeMap<String, String>, pub unset: Vec<String>, pub key: Option<KeyId> }
```

## A per-run key

```rust
pub struct ProxyKey {
    pub id: String,            // the first 8 hex digits of its sha256
    pub terminal: String,      // `<machine>/<name>`: the run it was made for
    pub provider: String,
    pub account: String,       // the account whose token signs its calls now
    pub created: u64,
    pub state: KeyState,       // Live | Revoked | Ended (its terminal exited or was removed)
}
```

The key itself is `shk-` and 48 hex digits from `/dev/urandom`. It exists in three places: the remote agent's
environment, the request it sends, and the proxy's memory for the length of one comparison.

## The API

| method | params → result | |
|---|---|---|
| `account.ls` | `{provider?}` → `[Account]` | what shed last read: per provider `default` first, then the order `auto` picks; a reading older than the record's `usage.fresh` is read again in the background |
| `account.add` | `{provider, name, folder?, token_file?}` → `Account` | makes the empty folder; `folder` adopts an existing one by link; `token_file` (a path on this host, 0600) is copied in; the same call twice is one account |
| `account.login` | `{provider, name, token?}` → `{terminal}` | the record's `login` (with `token`: its `token.command`) in a redacted terminal with the account's folder set |
| `account.status` | `{provider, name}` → `Account` | runs the status command and the usage probe now |
| `account.rm` | `{provider, name}` → `{}` | the record's `logout`, then the folder (an adopted one: its link), the token and the readings; refused for `default` and while a terminal runs on it |
| `proxy.keys` | `{}` → `[ProxyKey]` | newest first |
| `proxy.revoke` | `{id}` → `{}` | the key stops signing at once |
| `proxy.relay` | `{}` → `{port, socket}` | server to server, over a held link: the machine's relay port and the socket path the next forward binds |
| `machine.add`, `machine.rm` | `{name, target}`, `{name}` → `{}` | what `shed machine add\|rm` does, as methods; adding the same pair twice is one machine |
| `terminal.procs` | `{name}` → `[Proc]` | the terminal's process tree, parents first; `<machine>/<name>` asks that machine |

Changed shapes:

```rust
pub struct RunParams   { /* … */ pub account: Option<String> }   // a name, or "auto"; none: shed sets nothing
pub struct TerminalInfo { /* … */ pub account: Option<String>, pub redacted: bool }
pub enum EventKind     { /* … */ Account { account: String, was: Option<String>, why: String } }  // "start", "limit"

pub struct FileParams  { /* … */ pub mode: Option<u32> }         // file.write: created at this mode, then renamed in
pub struct ExecParams  { /* … */ pub env: BTreeMap<String, String> }   // memory only, like run's
pub struct StatsParams { /* … */ pub machine: Option<String>, pub top: Option<u32> }

pub struct Sample {
    /* … */
    pub mem_free_mb: u64, pub mem_total_mb: u64,
    pub swap_out_pages: u64,           // since boot; a caller takes the rate from two samples
    pub disk_total_mb: u64,
}
pub struct MachineStats { /* … */ pub top_cpu: Vec<Proc>, pub top_mem: Vec<Proc> }   // `top` of each

pub struct Proc {
    pub pid: i32, pub ppid: i32, pub depth: u32,
    pub cpu_pct: u64, pub rss_mb: u64, pub elapsed_s: u64,
    pub command: String,
    pub terminal: Option<String>,      // machine.stats: the terminal whose tree holds it
    pub own: bool,                     // terminal.procs: the agent's own process, not work it started
}

pub struct Link {
    /* state, since, error */
    pub down_since: Option<u64>,       // when it last left `up`; unlike `since`, a retry does not move it
    pub hello: Option<Hello>,          // the shed there: version, build, protocol
    pub os: Option<String>, pub arch: Option<String>, pub home: Option<PathBuf>,
    pub proxy: Option<u16>,            // its relay port, while the forward to this host's proxy is up
}
```

New error codes:

| code | when |
|---|---|
| `NO_ACCOUNT` | no such provider or account; `auto` with no named account; an agent whose record has no `config_dir_var` |
| `ACCOUNT_SIGNED_OUT` | the named account, or every `auto` candidate, is signed out; the message has the login call |
| `ACCOUNT_LIMITED` | a run started on a limited account, or every `auto` candidate is; the message has the first reset |
| `ACCOUNT_LOCAL_ONLY` | a run on a machine whose account has no token, or whose record has no `[proxy]` |
| `ACCOUNT_SETUP` | the account's folder holds one of the record's `setup_files`; the message names it |
| `NO_PROXY` | the machine's `Link::proxy` is none, or the shed there speaks a protocol under 5 |

## The record's new keys

`crates/shed-catalog/providers/claude.toml`, added lines. The values are today's facts from cooee
(`providers/claude.rs`) and drover (`providers/claude.rs`, `proxy.rs`), except `token.prefix`.

```toml
setup_files = ["CLAUDE.md", "settings.json", "rules", "agents", "commands", "skills", "plugins"]
session_files = ["projects/*/{session}.jsonl"]       # under the config folder; copied on an account switch
scrub = ["CLAUDE_CODE_OAUTH_TOKEN"]                  # besides `cache.api_vars`: never inherited by a run on an account

[status_read]            # without this table: exit 0 is signed in, the first output line is the status
format = "json"
signed_in = "loggedIn"
who = ["email", "orgName"]
how = "authMethod"

[usage]
command = "claude -p /usage"
env = { TZ = "UTC" }
unset = ["CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC"]
fresh = 300              # seconds a reading stands
hold = 3600              # seconds a limit mark stands when no reading gives a reset time

[[usage.windows]]
id = "5h"
line = "Current session:"            # the line starts with this
used = "% used"                      # the number before this marker
resets = "resets "                   # the time after this marker

[[usage.windows]]
id = "7d"
line = "Current week (all models):"
used = "% used"
resets = "resets "

[token]
command = "claude setup-token"
prefix = "sk-ant-oat"                # the token on the screen starts with this; slice 5 confirms it on a live run
days = 365

[proxy]
upstream = "https://api.anthropic.com"
paths = ["/v1/"]                     # a request outside these is refused before any token is read
base_url_var = "ANTHROPIC_BASE_URL"
key_var = "ANTHROPIC_AUTH_TOKEN"
auth = "authorization: Bearer {token}"
append = { "anthropic-beta" = "oauth-2025-04-20" }   # added to the header's list when the request lacks it
```

`codex.toml` gets nothing but what it has (`CODEX_HOME`, login, status, logout): exit 0 is signed in, no usage
probe, no proxy. `gemini.toml` gets `config_dir_var = "GEMINI_CLI_HOME"` and cooee's login and status commands.

The readers are pure functions beside `Provider` in `shed-proto`, like `limit_hit` today:

```rust
impl Provider {
    pub fn read_status(&self, ok: bool, stdout: &str, stderr: &str) -> Status;   // signed_in, line, who
    pub fn read_usage(&self, stdout: &str, now: u64) -> Option<Usage>;
}
pub fn when(text: &str, now: u64) -> Option<u64>;
```

`when` reads four forms, all as UTC: unix seconds, `3pm` or `3:30pm` (the next such time), `Nov 16 at 2am`
(the nearest such date) and `in 2h 5m`. A usage text in another form needs a new form in `when`, which is
code; everything else about a new provider is its `.toml`.

`redact` moves from cooee-core `login.rs` to `shed-core/src/redact.rs` unchanged in behaviour: a run of 20 or
more token characters holding a letter and a digit becomes `[redacted]`, and so does the value of a link's
`token`, `secret`, `password` or `key` parameter.
