# Shapes: config, records, the log, demand, rows, and shed's two machine changes

Times are unix seconds, UTC. Money is USD. A name (`VmName`, `PoolName`, `GroupName`, `Size`, `RecordName`,
`Tag`, a group's `service`) is checked against `[A-Za-z0-9_-]+` when it is parsed, so the only strings ever
filled into a verb are of that set. A repo is checked as `owner/name`.

## On disk

On the main host only. The helper has its own folders beside shed's, and shares none of shed's files.

```
<config>/shed-cloud/config.toml              the owner's: caps, pools, named VMs, runner groups, the git record
<config>/shed-cloud/providers/<name>.toml    the owner's record, replacing the built-in of that name
<config>/shed-cloud/git/<name>.toml          the same, for a git record
<config>/shed-cloud/images/<file>            a pool's build script
<config>/shed-cloud/secrets/<NAME>           a provider's key, the git record's key (0600, folder 0700)
<state>/shed-cloud/log.jsonl                 every change, one line each
<state>/shed-cloud/demand/<by>.json          a publisher's last demand, with the time it arrived
<state>/shed-cloud/plan.json                 the last sizing's answer, for `ls`
<state>/shed-cloud/lock/<vm>                 held while a command or the pass works on that VM
```

`<config>` and `<state>` are where shed's own are: `~/Library/Application Support` on macOS, `$XDG_CONFIG_HOME`
and `$XDG_STATE_HOME` elsewhere. A pinned host key is not here: it is in shed's machine entry.

## Config

```toml
[caps]                           # every pool together; only an edit of this file raises them
max_vms = 4                      # pool VMs at once
budget_usd_month = 150           # required once a pool exists: pool VMs and image builds
max_per_tag = { gpu = 1 }        # VMs at once of the pools that carry the tag
demand_ttl_minutes = 15          # the default

[pool.dev]                       # disposable lane hosts, sized from demand
provider = "boat"                # a record
size = "large"                   # a key of the record's [rate_usd_hour]
tags = ["linux", "docker"]       # a demand entry fits when all its tags are here
slots = 2                        # lanes at once on one VM
max_lanes = 2                    # the default: terminals over its life, then it drains
idle_minutes = 20                # the default
max_life_hours = 12              # the default
max_vms = 3

[pool.dev.image]
build = "images/dev.sh"          # under the helper's config folder
key = ["drover", "image-key", "app"]   # optional: a program and its arguments, no shell

[vm.boat1]                       # a named VM: the owner starts and stops it
provider = "boat"
size = "large"
budget_usd_month = 40            # required: a named VM with no budget does not load
max_hours_day = 6                # optional
idle_stop_minutes = 20           # the default
machine = "boat1"                # the default: the VM's name

[runners.app]                    # a group of runner VMs: GitHub runners, never shed machines
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

[git]                            # optional: the token git and gh use on a VM
record = "github-app"
refresh_minutes = 45             # the default
app = "123456"                   # every other key fills the record's lines
installation = "7654321"
repos = ["owner/app", "owner/lib"]
```

```rust
pub struct Config {
    pub caps: Caps,
    pub pool: BTreeMap<PoolName, Pool>,
    pub vm: BTreeMap<VmName, Named>,
    pub runners: BTreeMap<GroupName, Group>,
    pub git: Option<Git>,
}

pub struct Caps {
    pub max_vms: u64,
    pub budget_usd_month: f64,
    pub max_per_tag: BTreeMap<Tag, u64>,
    pub demand_ttl_minutes: u64,
}

pub struct Pool {
    pub provider: RecordName,
    pub size: Size,
    pub tags: BTreeSet<Tag>,
    pub slots: u64,
    pub max_lanes: u64,
    pub idle_minutes: u64,
    pub max_life_hours: u64,
    pub max_vms: u64,
    pub image: Image,
}

pub struct Image { pub build: PathBuf, pub key: Vec<String> }

pub struct Named {
    pub provider: RecordName,
    pub size: Size,
    pub budget_usd_month: f64,
    pub max_hours_day: Option<f64>,
    pub idle_stop_minutes: u64,
    pub machine: Option<String>,
}

pub struct Group {
    pub provider: RecordName,
    pub repo: Repo,
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

pub struct Git { pub record: RecordName, pub refresh_minutes: u64, pub fill: BTreeMap<String, Fill> }
pub enum Fill { Name(String), Repos(Vec<Repo>) }
```

An unknown key is an error that names it. A `size` the record does not price is an error at load. So is a
pool with no `[caps]`, and a named VM called `<pool>-<digits>`: those names are the pool's.

## A provider record: the exec contract

A record is the whole provider boundary. Each verb is a `sh -c` template run on the main host. Before it runs,
`{name}`, `{id}`, `{size}`, `{snapshot}` and `{ttl}`, and a group's `{snapshots}` and `{service}`, are filled
in. `{id}` is the record's own `id` line, or the id the log holds for a VM a `create` named. `{snapshot}` is a
pool's image name, or for a named VM the name in its last `snapshot` line (`<vm>-saved` when it saves one). `{ttl}` is seconds: twice a pool's `max_life_hours`, a
group's `ttl_hours`, 0 for a named VM.

```toml
cli = "boat"                               # optional: doctor checks each subcommand a verb uses against `<cli> --help`
key_env = "BOAT_API_KEY"                   # optional: the key's variable; without it the provider's CLI holds its own login
identity = "~/.ssh/<key file>"             # optional: the ssh key a new VM accepts
never = ["~/.git-credentials", "…"]        # paths that may not exist on a VM when an image is saved

[verbs]
id = '''…'''                               # prints the provider's id for {name}; nothing when there is none
create = "…"                               # a fresh VM
create_from = "…"                          # optional: a new VM from {snapshot}, with none of the account's secrets
start = "…"
stop = "…"
state = '''…'''
address = '''…'''
ready = '''…'''                            # optional: once after a create
snapshot = '''…'''                         # optional: saves the VM as {snapshot}
snapshot_rm = '''…'''                      # optional: removes {snapshot}
destroy = '''…'''
exec = '''…'''                             # optional for a lane-host record, required for a group's

[rate_usd_hour]
small = 0.018
large = 0.072

[runner]                                   # a group's record only: lines run inside the VM, through `exec`
revive = "…"                               # reads the token from stdin, restarts {service}
token = "…"                                # reads the token from stdin
drain = "…"
names = "…"                                # prints the name prefix of this VM's runners
```

| verb | stdin | must print | exit |
|---|---|---|---|
| `id` | the key | the id, or nothing | 0 |
| `create`, `create_from` | the key | nothing, or the new VM's id as its last line (a record without `id`) | 0 |
| `start`, `stop`, `destroy`, `ready`, `snapshot`, `snapshot_rm` | the key | anything | 0 |
| `state` | the key | `running` when it is up; nothing when there is no such VM; any other word otherwise | 0 |
| `address` | the key | line 1 `user@host port`, line 2 the host key (`ssh-ed25519 AAAA…`) | 0 |
| `exec` | the key, then the token when the step has one | the command's stdout; the command is in `$C` | the command's |

Rules of the contract:

- The helper's wrapper reads the key and the token as the first lines of stdin and exports them, for that verb's
  shell only, under `key_env` and as `$RT`.
- A non-zero exit fails the verb, with its stderr (secrets cut out) as the error.
- A verb that uses `{id}` does not run when `id` printed nothing (`NO_VM`).
- A record without a verb a command needs refuses that command and names the verb. A pool needs `create_from`,
  `snapshot`, `snapshot_rm` and `destroy`.
- A verb has 5 minutes, `exec` 10.

`boat.toml` is drover's built-in recipe with `status` renamed `state`, its verbs under `[verbs]`, the snapshot's
name and the time limit as fill-ins, `--no-env` on `create_from`, and an `exec` line through the CLI's own ssh
(each checked against the CLI's `--help` in C3, as drover's lines were in #635). `boat-api.toml` is the recipe's
`runner_create`, `runner_state`, `runner_delete` and `runner_exec` as `create`, `state`, `destroy` and `exec`,
and its four VM-side lines under `[runner]`.

A second provider is a second file. Nothing else changes.

## A git record

```toml
key_env = "GITHUB_APP_KEY"                 # the secret `mint` reads from stdin
mint = '''…'''                             # sh -c on the main host; prints the token, then its expiry
install = '''…'''                          # sh -c on the VM, through shed's `exec`; the token on stdin
remove = '''…'''                           # sh -c on the VM: before an image is saved
```

`github-app.toml` ships: `mint` signs a request with the App's key and asks GitHub for a one-hour token for
`{repos}` under `{app}` and `{installation}`; `install` makes git's credential helper and `gh` on the VM use
it. The token is a `Secret` from the moment `mint` prints it.

## The log

```rust
pub struct Line { pub at: u64, #[serde(flatten)] pub event: Event }

#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Event {
    Start { vm: VmName, of: Kind, pool: Option<PoolName>, new: bool },   // a create or a resume was asked for; spend runs from here
    Setup { vm: VmName, id: Option<String> },               // the provider says running
    Up { vm: VmName, address: Address, key: HostKey },      // set up; the machine is added
    Repin { vm: VmName, was: HostKey, key: HostKey },       // a start, or `repin`, took another key
    KeyChanged { vm: VmName, key: HostKey },                // refused: the hold
    Quiet { vm: VmName },
    Busy { vm: VmName },
    Lane { vm: VmName, terminal: String },                  // a terminal shed listed there for the first time
    GitToken { vm: VmName },                                // a fresh one was installed; no value
    Drain { vm: VmName, why: Why, end: End },               // `machine.drain` answered
    Stop { vm: VmName, why: Why, note: String },            // spend ends here; a named VM
    Removed { vm: VmName, why: Why, note: String },         // spend ends here; destroyed
    Snapshot { vm: VmName, name: String },
    Image { pool: PoolName, key: String, snapshot: String },
    ImageFailed { pool: PoolName, key: String, step: String, note: String },
    ImageBad { pool: PoolName, snapshot: String },          // two VMs in a row failed setup from it
    RunnerAdd { group: GroupName, vm: VmName, prefix: String, usd_day: f64 },
    RunnerRevive { group: GroupName, vm: VmName },
    RunnerTokens { group: GroupName, sent: u64, failed: Vec<VmName> },
    RunnerHealth { group: GroupName, online: u64 },         // logged when the number changes
    RunnerRm { group: GroupName, vm: VmName, runners: u64 },
}

pub enum Kind { Named, Pool, Image, Runner }
pub enum Why { Idle, Lanes, Age, Budget, Asked, Outside, Failed }
pub enum End { Stop, Destroy }
pub struct Address { pub user: String, pub host: String, pub port: u16 }   // parsed from `user@host port`
pub struct HostKey(String);                                                 // `<type> <base64>`, checked
```

A line is appended whole, under the VM's lock. Everything else is a function of the lines:

```rust
pub fn state(lines: &[Line]) -> State;                                      // the last of start, setup, up, drain, stop, removed
pub fn quiet_since(lines: &[Line]) -> Option<u64>;                          // the last of up, busy, quiet, when it is quiet
pub fn lanes(lines: &[Line]) -> u64;                                        // `lane` lines since the last `start`
pub fn spent(lines: &[Line], rate_usd_hour: f64, now: u64) -> Spent;        // start → stop or removed, cut to the month and day
pub fn over(vm: &Named, spent: &Spent) -> Option<String>;                   // "spent $40.02 of $40 this month"
pub fn due(pool: &Pool, lines: &[Line], now: u64) -> Option<Why>;           // a pool VM's limits: lanes, idle, age
pub fn pool_vms(lines: &[Line], pool: &PoolName) -> Vec<PoolVm>;            // start less removed, each with its state and lanes; sizing adds live terminals from `machine.stats`
pub fn next_name(lines: &[Line], pool: &PoolName) -> VmName;                // `<pool>-<n>`, n above every one the log has
pub fn image(lines: &[Line], pool: &PoolName) -> Option<String>;            // the newest `image` snapshot not marked bad
pub fn group_vms(lines: &[Line], group: &GroupName) -> Vec<VmName>;         // runner-add less runner-rm

pub struct Spent { pub usd_month: f64, pub hours_today: f64 }
```

## Demand

```rust
pub struct Demand { pub waiting: Vec<Want> }                // stdin of `demand --by <who>`; stored with `at`
pub struct Want { pub tags: BTreeSet<Tag>, pub lanes: u64 }

pub struct Plan { pub created: Vec<VmName>, pub short: Vec<Short> }         // what `demand --json` prints
pub struct Short { pub tags: BTreeSet<Tag>, pub lanes: u64, pub why: Cap, pub note: String }
pub enum Cap { NoPool, NoImage, PoolMax, TagMax, MaxVms, Budget }
```

`--by` is a checked name. An entry with no tags, no lanes or an unknown key is `BAD_DEMAND`; nothing is stored.

## What `ls --json` prints

```rust
pub struct Listed { pub vms: Vec<VmRow>, pub pools: Vec<PoolRow>, pub groups: Vec<GroupRow>, pub caps: CapsRow }

pub struct VmRow {
    pub name: VmName,
    pub kind: Kind,                        // named | pool | image | runner
    pub provider: RecordName,
    pub size: Size,
    pub state: State,                      // absent | starting | setup | running | draining | stopped
    pub since: u64,
    pub hold: Option<Hold>,                // budget | key_changed | unreachable | drain_stuck
    pub note: String,                      // one line a person can read: "lanes 2 of 2; 1 terminal left"
    pub spent_usd_month: f64,
    pub machine: Option<String>,           // while it is a shed machine
    pub address: Option<String>,           // running or draining: `user@host:port`
    pub quiet_since: Option<u64>,
    pub pool: Option<PoolName>,            // pool, image
    pub lanes: Option<u64>,                // pool: lanes it has had
    pub group: Option<GroupName>,          // runner
    pub hours_today: Option<f64>,          // named
    pub budget_usd_month: Option<f64>,     // named
    pub max_hours_day: Option<f64>,        // named
    pub idle_stop_minutes: Option<u64>,    // named
}

pub struct PoolRow {
    pub name: PoolName,
    pub tags: BTreeSet<Tag>,
    pub slots: u64,
    pub max_lanes: u64,
    pub vms: u64,                          // every state but absent
    pub max_vms: u64,
    pub image: Option<String>,             // the snapshot new VMs come from
    pub image_at: Option<u64>,
    pub building: bool,
    pub waiting: u64,                      // lanes of the fresh demand that fall to this pool
    pub short: Vec<Short>,                 // as the last sizing left it
    pub note: String,
}

pub struct CapsRow { pub vms: u64, pub max_vms: u64, pub spent_usd_month: f64, pub budget_usd_month: f64 }

pub struct GroupRow {
    pub name: GroupName,
    pub repo: Repo,
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

`ls` asks no provider, no GitHub and no shed: it is the log, the config, the demand files, the last plan and
the clock, so drover can run it every pass. A group's `online` is the last `runner-health` line, which the pass writes when
the number changes.

Refusals (exit 3): `OVER_BUDGET`, `KEY_CHANGED`, `NO_VM`, `NOT_NAMED` (`start` or `stop` on a pool VM: it has
`drain`), `BAD_DEMAND`, `IMAGE_DIRTY`, `RUNNERS_CAP`, `RUNNERS_LAST`, `SECRET_FILE_OPEN`, `SHED_OLD`,
`NOT_EMPTY` (`import` over a log with lines), `BUSY` (the VM's lock is held past the caller's wait).

## shed's two machine changes

Both are in shed's server, and nothing in either is about VMs.

### A pinned key (slice C1)

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
| is told `repin`, or starts a named VM whose key differs from the last `up` | the same call, and logs `repin` |
| ends a VM, or finds it gone | `machine.rm` |

### Draining (slice C2)

```rust
pub struct DrainParams {                   // `machine.drain`, routed by `machine` like `machine.stats`
    pub on: bool,                          // false lifts it
    pub why: Option<String>,               // shown with the refusal and in the stats
    pub machine: Option<String>,
}

pub struct Draining { pub since: u64, pub why: Option<String> }

pub struct MachineStats {
    // … as now …
    pub draining: Option<Draining>,        // new: this shed takes no new terminal
}
```

`ErrorCode` gains `Draining` (`DRAINING`): `run` on a draining shed. The state is one small file beside the
server's own state, read at its start. What it refuses and shows is in [pool.md](pool.md).

## From drover's config and log (the import)

drover writes these; `shed-cloud import` checks and takes them. Today's hosts come over as named VMs.

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
| `cloud-start` | `start`, `of: named` |
| `cloud-up` | `up`, with the address and the key from `~/.ssh/herdsman/<n>.known` |
| `cloud-stop` | `stop`; `why` from its text (`idle …`, `spent …`, `owner`, `gone` and `stopped outside drover`, `start failed …`) |
| `cloud-repin` | `repin` |
| `cloud-snapshot` | `snapshot`, `name: drover-<n>`, as drover's recipe saved it |
| `runners-add`, `runners-remove`, `runners-revive`, `runners-tokens` | `runner-add`, `runner-rm`, `runner-revive`, `runner-tokens` |
| `cloud-abort`, `runners-status`, `runners-retire-local`, `host-sync`, `host-bootstrap` | not carried |

Only this UTC month's `cloud-*` lines matter to spend; older ones may be left out. The last `cloud-snapshot` of
a VM is carried whatever its age, since it decides `create_from`. `[pool.<name>]`, `[caps]` and `[git]` have no
old form: the owner writes them, and `shed-cloud doctor` says what is missing.
