# Shapes: installs, VMs, secrets, slots, the sandbox, balances

Wire types live in `shed-proto`, one area file each (`install.rs`, `vm.rs`, `secret.rs`, `slot.rs`,
`sandbox.rs`). Times are unix seconds. Nothing is dropped or retyped; every new field has a default. `machine`
on a method means "run it there, over the held link", as on `exec` today; a method without it runs on the main
host only.

## On disk

Beside shed's state file, on the main host:

```
vms.json                     each VM shed made or adopted: provider, id, size, image, labels, last state; no secret
vms/<name>.known             a VM's pinned host key, as a known_hosts line
secrets/<NAME>               shed's own secrets on Linux (0600, folder 0700); on macOS they are Keychain items
```

On every machine, beside its shed's state file:

```
slots.json                   the limits `slot.limits` set; the line itself is in memory
sandbox/<terminal>.json      a sandboxed terminal's rules, removed with the terminal
inside/inside.sock           the inside socket (0600), in a folder of its own
```

An install has no file of shed's: `<dest>`, `<dest>.new` and `<dest>.prev` are its state.

## Installs

```rust
pub struct Source {
    pub tool: String,          // "drover": the asset is `<tool>-<arch>-<os>`, as all three repos name it
    pub releases: String,      // `<releases>/<tag>/<asset>` and `<releases>/<tag>/SHA256SUMS`; https
    pub tag: String,           // "build-1a2b3c4", "build-staging-1a2b3c4": the caller's choice
}
pub struct Release { pub tag: String, pub assets: BTreeMap<String, String> }   // asset name → sha256

pub struct Stage {
    pub machine: Option<String>,
    pub source: Source,
    pub build: String,              // `<dest>.new --version` must contain it
    pub dest: PathBuf,              // the live binary; `~` is that machine's home
    pub gate: Vec<String>,          // arguments run on `<dest>.new`; a non-zero exit unstages it
    pub identifier: Option<String>, // macOS: a Developer ID signature with this identifier is required
}
pub struct Swap { pub machine: Option<String>, pub dest: PathBuf, pub build: String, pub links: Vec<PathBuf> }

pub struct Install { pub live: Option<Build>, pub staged: Option<Build>, pub prev: Option<Build> }
pub struct Build { pub version: String, pub signed: Option<bool> }   // its `--version` line; signed: macOS only
```

| method | params → result | |
|---|---|---|
| `install.find` | `Source` → `Release` | reads `SHA256SUMS` on the main host and keeps it in memory; none there yet is `NO_RELEASE`. `stage` reads it the same way when no `find` has |
| `install.ls` | `{machine?, dest, identifier?}` → `Install` | runs `--version` on the three files |
| `install.stage` | `Stage` → `Install` | on the machine: download to `<dest>.new.part`, sha256 against the main host's sum, mode 755, rename to `.new`, version, signature, gate. A failed check removes the file and is `UNSTAGED`. The build already staged: nothing is fetched |
| `install.swap` | `Swap` → `Install` | `.prev` becomes a hard link to the live binary, `.new` is renamed over it, each of `links` is made to point at it. `.new` is not `build`: `UNSTAGED`. Live is already `build`: nothing |
| `install.back` | `{machine?, dest}` → `Install` | `.prev` renamed over the live binary |

`UNSTAGED` carries `data.check`: `asset` (the release has none for that OS and CPU), `download`, `checksum`,
`version`, `signature` or `gate`, and the output's last lines. `dest` equal to a shed binary is `INVALID_PARAMS`.

## Machines

```rust
pub struct MachineAdd { pub name: String, pub target: String, pub install: bool }   // #256's, plus `install`

pub struct SetupParams { pub machine: String, pub steps: Vec<Step> }
pub enum Step {
    Agent { id: String },                                           // the record's `install`, then its adapter registered
    Sh { what: String, check: String, install: String, timeout_s: Option<u64> },
}
pub struct StepDone { pub what: String, pub did: Did, pub code: Option<i32>, pub tail: String }
pub enum Did { There, Installed, Failed }                           // `tail`: the last 20 lines of a failed step
```

`machine.add` with `install`: over plain ssh, the one place shed runs a command there without a shed. It reads
the machine's OS and CPU, stages and swaps shed's own release at `~/.local/bin/shed` with the checks above,
then runs `shed install` there. With a shed already there it only adds the machine. `machine.setup` runs each
step on the machine's shed, with `~/.local/bin` and `~/.cargo/bin` on its `PATH`: `check` passes, the step is
`There`; else `install` (30 minutes at most unless `timeout_s`), then `check` again. It stops at the first
`Failed` and returns the steps so far.

## VMs

A provider record, `crates/shed-catalog/vms/<name>.toml`; a file of the same name in `<config>/vms/` replaces it.
`boat.toml` and `boat-api.toml` are drover's `assets/recipes/boat.toml` split in two; `container.toml` is
cooee-desk's calls.

```toml
# vms/boat.toml: a cloud host, named, reached over ssh
name = "boat"
key = "BOAT_API_KEY"                 # shed's secret of that name, exported to each verb from stdin; optional
identity = "~/.ssh/ascii_box_ed25519"
[verbs]                              # `sh -c` templates: {name} {id} {size} {image} and the caller's {vars}
id = '''boat list --all --json --no-update | jq -r 'first(.sandboxes[]|select(.name=="{name}"))|.id' '''
create = "boat new --type {size} --name {name} --no-auto-stop --no-update >/dev/null"
create_from = "boat new --type {size} --name {name} --from shed-{name} --no-auto-stop --no-update >/dev/null"
start = "boat resume {id} --no-update >/dev/null"
stop = "boat stop {id} --no-update >/dev/null"
state = "…"                          # prints `running`, another word, or nothing when there is no such VM
address = "…"                        # prints `user@host port`, then the host key
ready = "boat ssh {id} --no-update true"
snapshot = "boat snapshot {id} shed-{name} --no-update"
destroy = "boat delete {id} --yes --no-update"
[rate_usd_hour]
small = 0.018
default = 0.036
```

```toml
# vms/boat-api.toml: a runner VM, made by the HTTP API; `create` prints its id, which is its name
name = "boat-api"
key = "BOAT_API_KEY"
[verbs]
create = "curl -sf -X POST … -d '{\"type\":\"{size}\",\"ttlSeconds\":{ttl},\"snapshots\":{snapshots}}' | …"
state = "…"
destroy = "…"
exec = "…"                           # posts $C; the caller's stdin replaces {stdin} in it
```

```toml
# vms/container.toml: a Linux desk on this Mac
name = "container"
network = "shed-desks"
[verbs]
prepare = "…"                        # makes the network with --internal; one that exists and is not host-only fails
create = "container create --name {name} --label shed.vm=1 --cpus {cpus} --memory {memory}M --network shed-desks {image}"
start = "container start {name}"
stop = "container stop {name}"
state = "…"
destroy = "container delete --force {name}"
exec = 'container exec -i {name} sh -c "$C"'
```

A value filled into a template (`name`, `size`, `image`, each of `vars`) must match `[A-Za-z0-9._:/@-]{1,128}`;
anything else is `INVALID_PARAMS`. `{id}` is the `id` verb's output, or `name` when the record has none; a verb
that needs `{id}` and gets nothing does not run (`NO_VM`). `$C` and stdin are never filled into a template.

```rust
pub struct Vm {
    pub name: String,
    pub provider: String,
    pub id: Option<String>,
    pub state: VmState,                  // Gone | Stopped | Starting | Running
    pub since: u64,
    pub size: Option<String>,
    pub image: Option<String>,
    pub machine: Option<String>,         // the shed machine it is, when its record has `address`
    pub address: Option<String>,         // `user@host port`, as last read
    pub snapshot_at: Option<u64>,
    pub rate_usd_hour: Option<f64>,      // the record's rate for its size: a fact, not a budget
    pub labels: BTreeMap<String, String>,   // the caller's: drover's host, cooee's persona
}

pub struct VmCreate {
    pub provider: String,
    pub name: Option<String>,            // none: the id `create` prints
    pub size: Option<String>,
    pub image: Option<String>,
    pub from_snapshot: bool,             // `create_from`
    pub vars: BTreeMap<String, String>,  // {ttl}, {snapshots}, {cpus}, {memory}
    pub labels: BTreeMap<String, String>,
}
pub struct VmStart { pub name: String, pub repin: bool, pub probe: Vec<String> }   // probe: `host:port` it must not reach
pub struct VmExec { pub name: String, pub cmd: String, pub stdin: Vec<u8>, pub timeout: Option<f64> }
pub struct VmProvider { pub name: String, pub verbs: Vec<String>, pub rate_usd_hour: BTreeMap<String, f64> }
```

| method | params → result | |
|---|---|---|
| `vm.providers` | `{}` → `[VmProvider]` | the records, with the verbs each has |
| `vm.ls` | `{refresh?}` → `[Vm]` | what shed last saw; `refresh` asks each provider now and emits `vm` for a change |
| `vm.create` | `VmCreate` → `Vm` | `prepare`, then `create` (or `create_from`). A VM the provider already has under that name is adopted |
| `vm.start` | `VmStart` → `Vm` | `start`, then `state` every 10 s for 5 minutes at most, `ready` once after a create, `address`, the pin, the link; then each `probe` from inside. A failure stops the VM |
| `vm.stop`, `vm.destroy` | `{name}` → `Vm` | twice is once; `destroy` removes the pin and the machine |
| `vm.snapshot` | `{name}` → `Vm` | the record's `snapshot`; a record without one is `VM_VERB` |
| `vm.exec` | `VmExec` → `ExecResult` | the record's `exec` with `$C` set to `cmd`; stdout as bytes |

A VM machine has no line in `[machines]`: `links.rs` builds its ssh target from `address` and the pin
(`HostName`, `Port`, `User`, `UserKnownHostsFile`, `IdentityFile`, `IdentitiesOnly`) as options on its own
`ssh`. The pin is written at the first start; a different key later is `VM_KEY_CHANGED` unless `repin`.

## Secrets

Crate `shed-secrets`, linked by shed's server, cooee and drover:

```rust
pub struct Secret(String);               // no Debug, Display, Serialize or Clone; 1 to 8,192 bytes
impl Secret { pub fn new(v: String) -> Result<Self, Error>; pub fn expose(&self) -> &str }

pub struct Place {
    pub service: String,                 // macOS: the Keychain service ("shed-secret", "cooee-secret")
    pub prefix: String,                  // macOS: before each account name (cooee: its data folder's hash)
    pub files: PathBuf,                  // elsewhere: the folder of 0600 files
}
pub struct Store { /* a Place */ }
impl Store {
    pub fn open(place: Place) -> Self;
    pub fn get(&self, name: &str) -> Result<Option<Secret>, Error>;
    pub fn set(&self, name: &str, value: &Secret) -> Result<(), Error>;
    pub fn remove(&self, name: &str) -> Result<(), Error>;        // twice is once
    pub fn names(&self) -> Result<Vec<String>, Error>;
    pub fn adopt_files(&self) -> Result<Vec<String>, Error>;      // macOS: each file into the Keychain, read back, shredded
}
pub enum Error { Open(PathBuf), Locked, Unsigned, Refused(String), Name(String), Io(std::io::Error) }
```

`Open`: the file is wider than 0600 and was not read. `Locked`: the Keychain gave no answer in 5 seconds.
`Unsigned`: this program has no Apple-issued signature, so its next build could not read what it saved; nothing
is saved. A name is `[A-Za-z0-9._-]{1,128}`.

shed's own secrets, on the wire in `secret.rs`:

| method | params → result | |
|---|---|---|
| `secret.set` | `{name, value}` → `{}` | saved in shed's store; `value` is never logged, saved in the state file or echoed in an error |
| `secret.ls` | `{}` → `[{name, saved_at}]` | names only |
| `secret.rm` | `{name}` → `{}` | twice is once |

There is no `secret.get`. `shed secret set <NAME>` reads the value from stdin.

## Slots

```rust
pub struct Slot { pub kind: String, pub holder: String, pub cmd: String, pub wait_s: u64 }   // cmd: shown in `slot.ls`
pub struct Taken { pub admitted: bool, pub waited_s: u64 }
pub struct Ticket { pub kind: String, pub holder: String, pub cmd: String, pub since: u64, pub running: bool }
pub struct Slots { pub max: u32, pub kinds: BTreeMap<String, u32>, pub line: Vec<Ticket> }
pub struct SlotLimits { pub machine: Option<String>, pub max: u32, pub kinds: BTreeMap<String, u32> }

pub struct ExecParams { /* … */ pub slot: Option<Slot>, pub sandbox: Option<Sandbox> }
```

| method | params → result | |
|---|---|---|
| `slot.take` | `Slot` → `Taken` | answers when admitted, or with `admitted: false` once `wait_s` have passed; the slot ends with the connection |
| `slot.free` | `{}` → `{}` | ends this connection's slot early |
| `slot.ls` | `{machine?}` → `Slots` | the line in order: running first, then waiting |
| `slot.limits` | `SlotLimits` → `{}` | kept in `slots.json`; unset is 3 slots and no kind limit, drover's default |

Admission is drover's: the waiting tickets in order, each admitted while fewer than `max` run and fewer than
its kind's limit run of its kind; a ticket held back by its kind does not hold back the ones behind it.
`shed slot run <kind> [--wait 30m] [--late run|fail] -- <cmd…>` takes a slot, runs the command as its child with
its own stdin, stdout and exit code, and closes. Late with `run` (drover's `heavy`) it runs without a slot; with
`fail` (drover's `hold`) it exits 75.

## The sandbox

```rust
pub struct Sandbox {
    pub read: Vec<PathBuf>,       // besides the system's own folders
    pub write: Vec<PathBuf>,      // every write path is also read
    pub deny: Vec<PathBuf>,       // read denied even inside an allowed folder
    pub hosts: Vec<String>,       // domains it may reach; none: no network
    pub sockets: Vec<PathBuf>,    // unix sockets, besides the inside socket
    pub mach: Vec<String>,        // macOS mach services, besides srt's defaults
    pub tmp: Option<PathBuf>,     // its own temp folder; default one shed makes for the terminal
}
pub struct RunParams { /* … */ pub sandbox: Option<Sandbox> }
pub struct TerminalInfo { /* … */ pub sandboxed: bool }
```

The agent's own needs are keys of its record, with `{config}` the run's account folder (#256):

```toml
# providers/claude.toml
[sandbox]
tmp_var = "CLAUDE_CODE_TMPDIR"       # srt takes the run's temp folder from it
read = ["~/.local/share/claude"]
write = ["{config}", "~/.claude.json"]
hosts = ["*.anthropic.com", "claude.ai", "*.claude.ai"]
```

The inside socket serves `hello`, `heartbeat`, `state`, `emit`, `slot.take` and `slot.free`; any other method
there is `METHOD_NOT_FOUND`. A sandboxed terminal has `SHED_SOCKET` set to it, which the CLI honours.

## Balances

```toml
# providers/deepseek.toml
agent = "deepseek"
token_var = "DEEPSEEK_API_KEY"       # no config folder variable: an account of it is only a key
[balance]
url = "https://api.deepseek.com/user/balance"
auth = "Authorization: Bearer {token}"
currency = ["balance_infos", 0, "currency"]
total = ["balance_infos", 0, "total_balance"]
fresh = 600                          # seconds a reading stands
```

```rust
pub struct Account { /* #256 */ pub balance: Option<Balance> }
pub struct Balance { pub currency: String, pub total: f64, pub read_at: u64 }
impl Provider { pub fn read_balance(&self, body: &str, now: u64) -> Option<Balance>; }   // pure, beside `read_usage`
```

The probe is one `curl` whose config, with the header, goes over stdin, as #256's proxy sends a token. An
account of such a record is made with `account.add` and `token_file`, has no folder and no login, and is
`ready` while it has a key; a 401 marks the key `rejected`.

## Events and error codes

```rust
pub enum EventKind { /* … */ Vm { name: String, state: VmState, was: VmState, why: String } }   // "asked", "outside", "failed"
```

| code | when |
|---|---|
| `NO_RELEASE` | `install.find`: no `SHA256SUMS` at that tag |
| `UNSTAGED` | a check of `install.stage` failed, or `install.swap` found another build staged; `data.check` |
| `NO_VM` | no such VM or provider; a verb that needs an id got none |
| `VM_VERB` | the record has no such verb, or the verb exited non-zero; its stderr with the key cut out |
| `VM_START` | not `running` after 5 minutes; the VM was stopped |
| `VM_KEY_CHANGED` | the host key is not the pinned one; the VM was stopped; `repin` accepts the new key |
| `VM_REACHED` | a probed address answered from inside; the VM was stopped |
| `SECRET_STORE` | shed's store refused: a wide file, a locked Keychain, an unsigned shed; the message says which |
| `SANDBOX_MISSING` | `sandbox` asked for and no `srt` on that machine; nothing ran |
