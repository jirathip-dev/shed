# Shapes: config, provider records, the log, rows, and shed's machine entry

Times are unix seconds, UTC. Money is USD. A name (`VmName`, `PoolName`, `Size`, `RecordName`, a pool's
`service`) is checked against `[A-Za-z0-9_-]+` when it is parsed, so the only strings ever filled into a verb
are of that set.

## On disk

On the main host only. The helper has its own folders beside shed's, and shares none of shed's files.

```
<config>/shed-cloud/config.toml              the owner's: VMs, pools, limits
<config>/shed-cloud/providers/<name>.toml    the owner's record, replacing the built-in of that name
<config>/shed-cloud/secrets/<NAME>           a provider's key (0600, folder 0700)
<state>/shed-cloud/log.jsonl                 the helper's one file: every change, one line each
<state>/shed-cloud/lock/<vm>                 held while a command or the pass works on that VM
```

`<config>` and `<state>` are where shed's own are: `~/Library/Application Support` on macOS, `$XDG_CONFIG_HOME`
and `$XDG_STATE_HOME` elsewhere. A pinned host key is not here: it is in shed's machine entry.

## Config

```toml
[vm.boat1]                       # a lane host: a shed machine while it runs
provider = "boat"                # a record
size = "large"                   # a key of the record's [rate_usd_hour]
budget_usd_month = 40            # required: a VM with no budget does not load
max_hours_day = 6                # optional
idle_stop_minutes = 20           # the default
machine = "boat1"                # the default: the VM's name
before_stop = ["drover", "host", "save", "boat1"]   # optional: run on the main host before every stop

[runners.app]                    # a pool of runner VMs: GitHub runners, never shed machines
provider = "boat-api"
repo = "owner/app"
label = "app-boat"
size = "large"
slots = 3                        # runners per VM
max_vms = 5
usd_day = 30                     # the cap: VMs × rate × 24
ttl_hours = 0
snapshots = false
service = "ci-runner"            # the default
token_refresh_minutes = 30       # the default
```

```rust
pub struct Config { pub vm: BTreeMap<VmName, Host>, pub runners: BTreeMap<PoolName, Pool> }

pub struct Host {
    pub provider: RecordName,
    pub size: Size,
    pub budget_usd_month: f64,
    pub max_hours_day: Option<f64>,
    pub idle_stop_minutes: u64,
    pub machine: Option<String>,
    pub before_stop: Vec<String>,          // a program and its arguments; no shell; 10 minutes at most
}

pub struct Pool {
    pub provider: RecordName,
    pub repo: String,                      // `owner/name`, checked
    pub label: String,
    pub size: Size,
    pub slots: u64,
    pub max_vms: u64,
    pub usd_day: f64,
    pub ttl_hours: u64,
    pub snapshots: bool,
    pub service: String,
    pub token_refresh_minutes: u64,
}
```

An unknown key is an error that names it. A `size` the record does not price is an error at load.

## A provider record: the exec contract

A record is the whole provider boundary. Each verb is a `sh -c` template run on the main host. Before it runs,
`{name}`, `{id}`, `{size}` and the pool's `{ttl}`, `{snapshots}` and `{service}` are filled in; `{id}` is the
record's own `id` line, or the id the log holds for a VM a `create` named.

```toml
cli = "boat"                               # optional: doctor checks each subcommand a verb uses against `<cli> --help`
key_env = "BOAT_API_KEY"                   # optional: the key's variable; without it the provider's CLI holds its own login
identity = "~/.ssh/<key file>"             # optional: the ssh key a new VM accepts

[verbs]
id = '''…'''                               # prints the provider's id for {name}; nothing when there is none
create = "…"
create_from = "…"                          # optional: a new VM from this VM's snapshot
start = "…"
stop = "…"
state = '''…'''
address = '''…'''
ready = '''…'''                            # optional: once after a create
snapshot = '''…'''                         # optional
destroy = '''…'''
exec = '''…'''                             # optional for a host record, required for a pool's

[rate_usd_hour]
small = 0.018
large = 0.072

[runner]                                   # a pool's record only: lines run inside the VM, through `exec`
revive = "…"                               # reads the token from stdin, restarts {service}
token = "…"                                # reads the token from stdin
drain = "…"
names = "…"                                # prints the name prefix of this VM's runners
```

| verb | stdin | must print | exit |
|---|---|---|---|
| `id` | the key | the id, or nothing | 0 |
| `create`, `create_from` | the key | nothing, or the new VM's id as its last line (a record without `id`) | 0 |
| `start`, `stop`, `destroy`, `ready`, `snapshot` | the key | anything | 0 |
| `state` | the key | `running` when it is up; nothing when there is no such VM; any other word otherwise | 0 |
| `address` | the key | line 1 `user@host port`, line 2 the host key (`ssh-ed25519 AAAA…`) | 0 |
| `exec` | the key, then the token when the step has one | the command's stdout; the command is in `$C` | the command's |

Rules of the contract:

- The helper's wrapper reads the key and the token as the first lines of stdin and exports them, for that verb's
  shell only, under `key_env` and as `$RT`.
- A non-zero exit fails the verb, with its stderr (secrets cut out) as the error.
- A verb that uses `{id}` does not run when `id` printed nothing (`NO_VM`).
- A record without a verb a command needs refuses that command and names the verb.
- A verb has 5 minutes, `exec` 10.

`boat.toml` is drover's built-in recipe with `status` renamed `state`, its verbs under `[verbs]`, and an `exec`
line through the CLI's own ssh (checked against `boat ssh --help` in C2, as drover's lines were in #635).
`boat-api.toml` is the recipe's `runner_create`, `runner_state`, `runner_delete` and `runner_exec` as `create`,
`state`, `destroy` and `exec`, and its four VM-side lines under `[runner]`.

A second provider is a second file. Nothing else changes.

## The log

```rust
pub struct Line { pub at: u64, #[serde(flatten)] pub event: Event }

#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Event {
    Start { vm: VmName, new: bool },                        // a create or a resume was asked for; spend runs from here
    Setup { vm: VmName, id: Option<String> },               // the provider says running
    Up { vm: VmName, address: Address, key: HostKey },      // set up; the machine is added
    Repin { vm: VmName, was: HostKey, key: HostKey },       // a start, or `repin`, took another key
    KeyChanged { vm: VmName, key: HostKey },                // refused: the hold
    Quiet { vm: VmName },
    Busy { vm: VmName },
    Stop { vm: VmName, why: Why, note: String },            // spend ends here
    StopAborted { vm: VmName, note: String },               // before_stop failed; still running
    Snapshot { vm: VmName },
    Removed { vm: VmName },
    RunnerAdd { pool: PoolName, vm: VmName, prefix: String, usd_day: f64 },
    RunnerRevive { pool: PoolName, vm: VmName },
    RunnerTokens { pool: PoolName, sent: u64, failed: Vec<VmName> },
    RunnerHealth { pool: PoolName, online: u64 },           // logged when the number changes
    RunnerRm { pool: PoolName, vm: VmName, runners: u64 },
}

pub enum Why { Idle, Budget, Asked, Outside, Failed }
pub struct Address { pub user: String, pub host: String, pub port: u16 }   // parsed from `user@host port`
pub struct HostKey(String);                                                 // `<type> <base64>`, checked
```

A line is appended whole, under the VM's lock. Everything else is a function of the lines:

```rust
pub fn state(lines: &[Line]) -> State;                                      // the last of start, setup, up, stop, removed
pub fn quiet_since(lines: &[Line]) -> Option<u64>;                          // the last of up, busy, quiet, when it is quiet
pub fn spent(lines: &[Line], rate_usd_hour: f64, now: u64) -> Spent;        // start → stop spans, cut to the month and day
pub fn over(host: &Host, spent: &Spent) -> Option<String>;                  // "spent $40.02 of $40 this month"
pub fn pool_vms(lines: &[Line], pool: &PoolName) -> Vec<VmName>;            // runner-add less runner-rm

pub struct Spent { pub usd_month: f64, pub hours_today: f64 }
```

## What `ls --json` prints

```rust
pub struct Listed { pub vms: Vec<VmRow>, pub pools: Vec<PoolRow> }

pub struct VmRow {
    pub name: VmName,
    pub kind: Kind,                        // host | runner
    pub provider: RecordName,
    pub size: Size,
    pub state: State,                      // absent | starting | setup | running | stopped
    pub since: u64,
    pub hold: Option<Hold>,                // budget | key_changed | stop_aborted
    pub note: String,                      // one line a person can read: the stop's why, the hold's reason, "quiet 4 of 20 min"
    pub spent_usd_month: f64,
    pub hours_today: f64,
    pub budget_usd_month: Option<f64>,     // host
    pub max_hours_day: Option<f64>,        // host
    pub idle_stop_minutes: Option<u64>,    // host
    pub quiet_since: Option<u64>,          // host
    pub machine: Option<String>,           // host, while it is one
    pub address: Option<String>,           // host, running: `user@host:port`
    pub pool: Option<PoolName>,            // runner
}

pub struct PoolRow {
    pub name: PoolName,
    pub repo: String,
    pub label: String,
    pub slots: u64,
    pub vms: u64,
    pub online: u64,                       // as the last pass read GitHub's list
    pub read_at: u64,
    pub tokens_at: u64,
    pub usd_day: f64,
    pub note: String,
}
```

`ls` asks no provider and no GitHub: it is the log, the config and the clock, so drover can run it every pass.
A pool's `online` is the last `runner-health` line, which the pass writes when the number changes.

Refusals (exit 3): `OVER_BUDGET`, `KEY_CHANGED`, `STOP_ABORTED`, `NO_VM`, `RUNNERS_CAP`, `RUNNERS_LAST`,
`SECRET_FILE_OPEN`, `SHED_OLD`, `NOT_EMPTY` (`import` over a log with lines), `BUSY` (the VM's lock is held past
the caller's wait).

## shed's machine entry (slice C1)

The only change to shed's server, and nothing in it is about VMs.

```rust
pub struct MachineParams {                 // `machine.add`
    pub name: String,
    pub target: String,                    // `[user@]host`, or `ssh://[user@]host[:port]`, which ssh takes as it is
    pub host_key: Option<String>,          // new: the one key this machine may answer with
    pub identity: Option<PathBuf>,         // new: the one key file to offer
    pub replace: bool,                     // new: a taken name moves to these values, and its link is held again
}
```

```toml
[machines]
box = "user@box.example"
boat1 = { target = "ssh://user@vm.example:2222", host_key = "ssh-ed25519 AAAA…", identity = "~/.ssh/<key file>" }
```

For a machine with `host_key`, shed writes `<name> <key>` to a known-hosts file of its own in its control
folder and dials with `HostKeyAlias=<name>`, `UserKnownHostsFile=<that file>` and `StrictHostKeyChecking=yes`;
with `identity`, `IdentityFile=<it>` and `IdentitiesOnly=yes`. A machine with neither dials as it does today.
The same values again change nothing and re-dial nothing. Without `replace`, another target under a taken name
is `INVALID_PARAMS`, as now. `machine ls --json` shows both fields.

| when the helper | it calls |
|---|---|
| has set up a started VM | `machine.add { name, target, host_key, identity, replace: true }` |
| sees, at a pass, another address with the same key | the same call, and logs `up` |
| sees another key while the VM runs | nothing; logs `key-changed` |
| is told `repin`, or starts a VM whose key differs from the last `up` | the same call, and logs `repin` |
| stops or removes a VM, or finds it stopped outside | `machine.rm` |

## From drover's config and log (the import)

drover writes these; `shed-cloud import` checks and takes them.

| drover `fleet.toml` | `config.toml` |
|---|---|
| `[hosts.<n>]` `cloud`, `size`, `budget_usd_month`, `max_hours_day`, `idle_stop_minutes`, `machine` | `[vm.<n>]` `provider` and the same keys |
| `size = "medium"` on a record with no `medium` | `size = "default"`; the alias is not carried |
| `[limits] cloud_idle_stop_minutes` | written into each `[vm.<n>]` that set none |
| `[ci_runners]` | `[runners.<label>]`, `provider = "<name>-api"`, the same keys |
| `[ci_runners] vms` | one `runner-add` line per entry; the key is gone |
| `[ci_runners] local_label`, `local_stop` | not carried |
| the other `[hosts.<n>]` keys (`tags`, `runtime`, `max_working`, …) | stay drover's |

| drover event | log line |
|---|---|
| `cloud-start` | `start` |
| `cloud-up` | `up`, with the address and the key from `~/.ssh/herdsman/<n>.known` |
| `cloud-stop` | `stop`; `why` from its text (`idle …`, `spent …`, `owner`, `gone` and `stopped outside drover`, `start failed …`) |
| `cloud-repin`, `cloud-snapshot`, `cloud-abort` | `repin`, `snapshot`, `stop-aborted` |
| `runners-add`, `runners-remove`, `runners-revive`, `runners-tokens` | `runner-add`, `runner-rm`, `runner-revive`, `runner-tokens` |
| `runners-status`, `runners-retire-local`, `host-sync`, `host-bootstrap` | not carried |

Only this UTC month's `cloud-*` lines matter to spend; older ones may be left out. The last `cloud-snapshot` of
a VM is carried whatever its age, since it decides `create_from`.
