# ADR: hosts, VMs, installs, secrets and process limits are shed's; the policy stays in drover and cooee (#259)

## Summary

Decision: shed does the host-side work and takes no decision about it. It installs a checked release of any tool on any machine, makes and runs VMs from one data file per VM provider (cloud hosts, runner VMs and Apple container desks alike), keeps a line of slots for heavy commands on each machine, runs a terminal or a command inside a sandbox when asked, and reads an API-key provider's balance. Secrets get one implementation, a shed crate that keeps them in the Keychain on macOS and in 0600 files on Linux; each program reads only its own, in its own process, and no shed method ever returns one. drover and cooee keep every decision: which build and when, budgets and idle stops, runner registration, which commands are heavy, who gets a secret, a desk or a sandbox.
Slices: in shed, after #256's slices: (1) the secrets crate and shed's own secrets; (2) installs; (3) slots; (4) VMs, shed on a new machine and machine setup; (5) the sandbox option and the inside socket; (6) balance probes. Each is followed by one cooee or drover issue that moves its callers and deletes the old code. About 2,250 lines added in shed, about 2,400 removed from drover and cooee.
Open questions: (1) apps read their own secrets through the crate and shed returns none (recommended), or shed's server holds every secret and returns values to cooee and drover over its socket? (2) the Keychain needs the `security-framework` crate, which cooee already uses: add it (recommended)? (3) a sandboxed run reaches a second shed socket that only takes state reports, events and slots, so a sandboxed agent cannot start terminals: accept (recommended)? (4) one cap raise of about 2,300 lines in its own cap issue, sized by `drover cap shed` when filed.

**Status:** accepted by the owner (#259); the four open questions take the recommendations above (the issue's
amendment of 2026-11-22: each app reads its own secrets through `shed-secrets` and shed returns none,
`security-framework` on macOS only, a sandboxed run gets the limited inside socket, one cap raise of about 2,300
lines in its own cap issue). Proposed by the architect lane, 2026-11-22, read on shed `f5ca20d`, drover staging `5b2e0f796` and
cooee staging `c46881513`. It follows the owner's decision of the same day (shed owns machines, VMs, installs,
secrets and process limits; drover and cooee keep spend, when and which) and builds on the accepted accounts
ADR ([../accounts/adr.md](../accounts/adr.md), #256) without reopening it. The split, how the callers read, the
statecharts, the boundaries, the migration and the issues: [README.md](README.md). Types and record keys:
[shapes.md](shapes.md). The security model: [security.md](security.md).

## The question

The same host-side work is written two or three times, each copy with its own rules:

- **Installing a release.** shed updates itself (`crates/shed/src/update.rs`, 203 lines). drover updates itself
  on every host with shell lines over ssh (`update.rs` 627, `release.rs` 251). cooee updates itself on one host
  (`update.rs`, 276). All three download an asset, check it against `SHA256SUMS`, check `--version`, and rename
  it over the live binary.
- **VMs.** drover runs cloud hosts from a recipe of shell templates (`cloud.rs`, 669) and runner VMs from more
  templates in the same recipe (`runners.rs`, 359). cooee runs Apple `container` desks from Rust that calls a
  CLI (`cooee-desk`, 345). Both are create, start, stop, destroy, state and "run this inside".
- **Secrets.** drover keeps 0600 files and delivers them to a lane as a second 0600 file on the lane's host
  (`secret.rs`, 194). cooee keeps Keychain items on macOS and 0600 files on Linux (`secrets.rs`, 357).
- **Process limits.** drover has a slot line built on file locks (`heavy.rs`, 225), a process view from `ps`
  and `lsof` (`top.rs`, 210) and a sandbox wrap that ACP lanes do not get, because shed builds their command
  (`sandbox.rs`, 111).
- **Balances.** drover probes one provider's balance with its own key file (`usage.rs`).

shed already starts every process on every machine and holds the link to each. What exactly does it take over,
what stays a decision of the apps, and how do secrets stay where #256 put credentials: on the main host?

## Options

**Who rolls a tool out**

| | option | verdict |
|---|---|---|
| A | Leave three updaters | three copies of the checks; drover's runs as shell text over ssh |
| B | shed's own 5-minute job updates every registered tool on each machine by itself | each host would swap on its own clock; drover swaps all hosts in one pass with the main host last, and cooee rolls back when its port does not answer: that is "when", which stays in the apps |
| C | **shed has the verbs (find, stage, swap, put back); the app calls them** | chosen; shed's self-update calls the same code |

**How a VM provider is described**

| | option | verdict |
|---|---|---|
| D | A Rust trait with one implementation per provider | a new provider is code in shed; cooee-desk is this for one provider |
| E | **One record per provider: shell templates for the verbs, as drover's recipe has today** | chosen: `boat`, `boat-api` and `container` are three data files, and an owner's own file overrides one |
| F | Leave desks in cooee, move only cloud hosts | two VM lifecycles stay; the issue asks for both |

**Where a secret is kept and read**

| | option | verdict |
|---|---|---|
| G | shed's server holds every secret and returns values over its socket | every process of the user on the main host can ask the socket, so the Keychain's per-program access list protects nothing; today a cooee agent cannot read a persona's secret, and then it could |
| H | As G, with shed checking the caller's code signature | macOS only, and it has to go by the caller's audit token, since a process id can change hands; a signed `shed` or `drover` that a lane starts passes it anyway |
| I | shed holds them and returns none; a run names `secret:<NAME>` and shed fills it in | a caller can still read any secret back: run `printenv` with it. And cooee uses some in its own process (webhook checks, connector tokens, mail), so it would keep a store |
| J | **One crate in shed; each program reads its own secrets in its own process; shed's server keeps only the ones shed itself uses; no method returns one** | chosen: the Keychain item opens only to the signed program that saved it, a value crosses to shed only in the memory-only fields a run already has, and two store implementations become one |

**Heavy slots**

| | option | verdict |
|---|---|---|
| K | drover's lock folder, shared by path | cooee and CI would need drover's binary and its state folder to queue |
| L | **A line in each machine's shed; a slot is a lease held by a connection** | chosen: a holder that dies frees its slot when its connection closes, and the line is one `slot.ls` away |

**The sandbox**

| | option | verdict |
|---|---|---|
| M | The app wraps its command, as drover does | an ACP lane's command is built by shed, so it stays outside; the app resolves links on a host it is not on |
| N | **`sandbox` is an option of `run` and `exec`; shed writes the rules and wraps the command on the host that runs it** | chosen |

**What a sandboxed run may ask shed**

| | option | verdict |
|---|---|---|
| O | Nothing: no socket inside | no state reports from hooks and no slot for a heavy command |
| P | The full socket | `exec` runs outside the sandbox: one call and the agent is out |
| Q | **A second socket with six methods: hello, heartbeat, state, emit, take a slot, free it** | chosen |

## The decision

1. **Installs.** `install.find` reads a release's `SHA256SUMS` once on the main host. `install.stage` puts the
   asset for a machine's OS and CPU beside the live binary as `<dest>.new`, on that machine, and checks its
   sha256 against the sum the main host read, that its `--version` names the build, its Developer ID signature
   on macOS when the caller names an identifier, and the caller's gate (arguments run on the staged binary).
   `install.swap` keeps the live binary as `<dest>.prev` and renames the staged one over it. `install.back`
   puts `.prev` back. shed's self-update is the first caller. shed never chooses a build or a moment.
2. **shed on a new machine.** `machine.add` with `install` puts shed's own release there over ssh, checked the
   same way, and runs `shed install`. From then on shed updates itself there, as now. `machine.setup` runs a
   list of steps on a machine, each a check, an install and the check again; the list is the caller's, and an
   agent step takes its install line from the agent's record.
3. **VMs.** A VM provider is a record in `crates/shed-catalog/vms/`: shell templates for `create`, `start`,
   `stop`, `state`, `destroy`, and where it has them `address`, `snapshot`, `create_from`, `ready`, `exec`,
   `prepare`; the name of its key; its sizes and rates. `vm.create`, `start`, `stop`, `destroy`, `snapshot`,
   `exec` and `ls` run them on the main host. A start waits for `running`, reads the address and compares the
   host key with the pin. A VM with an address is a shed machine of the same name, with its ssh options held
   by shed, so nothing writes `~/.ssh/config`. Runner VMs are VMs of the `boat-api` record; desks are VMs of
   the `container` record, on a host-only network shed makes, probed at each start.
4. **Secrets.** Crate `shed-secrets`: a `Store` at a place the caller names (a Keychain service and prefix on
   macOS, a folder of 0600 files elsewhere), a `Secret` type that cannot be printed, every Keychain call limited
   to 5 seconds with prompts off. cooee and drover link it and read their own secrets in their own process, at
   the places they use today. shed's server keeps only shed's own (a VM provider's key), written by
   `secret.set`, listed by name, never returned. A value reaches a run in the fields that are already memory
   only: `run`'s `env` and MCP servers, `exec`'s `env` and `stdin`, `vm.exec`'s `stdin`, `send`'s text.
5. **Slots.** Each machine's shed keeps one line. `slot.take` waits in it and answers when a slot and the
   ticket's kind are free, or when the caller's wait has passed; the slot is held until that connection closes.
   `exec` takes a `slot`, and `shed slot run` runs a command as its own child, so the command stays in the
   lane's process tree and sandbox. The limits are numbers the app sets with `slot.limits`. The process tree is
   #256's `terminal.procs`; `slot.ls` adds who runs and who waits.
6. **The sandbox.** `run` and `exec` take `sandbox`: folders to read and write, hosts to reach. The shed on
   the machine that runs it adds the agent's own needs from its record and the account's folder, denies its
   own state and credentials, resolves links, writes the rules outside every writable folder and starts the
   command under `srt`. No `srt` there is an error, never a run without it. A sandboxed run is given the
   inside socket.
7. **Balances.** A provider record may have a `[balance]` table: the URL, the auth header and where the
   currency and the total are in the answer. An account may be only a key (#256's token file with no folder).
   shed probes it beside the usage probe and `account.ls` carries the reading.
8. **Policy stays out.** Every budget, wait, order, limit number, approval, alert and "which" is an argument an
   app passes or a decision it takes before it calls. The table in [README.md](README.md) names each.

What this reverses, said plainly:

- **drover #707: "drover keeps no secret store now", 0600 files only.** On macOS drover's secrets go back into
  the Keychain, now through shed's crate and under drover's Developer ID signature. Linux keeps the files.
- **drover `docs/hosts.md`: "drover reaches the host as `ssh <name> …` for everything"**, with a host block in
  `~/.ssh/config.d/herdsman` and an `Include` the owner adds. A cloud host is a shed machine; the block, the
  `Include` and the `sshd` drop-in bootstrap wrote are no longer needed.
- **drover's sandbox spike (#782): "Not covered: shed-acp lanes."** Covered: shed wraps the command it builds.
- **cooee decision 0003: desks are "one crate inside cooee", "no provider trait until a second backend".** The
  provider is a shed record; `cooee-desk` is deleted. Its go/no-go findings and conditions stand.
- **#256, one shape:** an account's `folder` may be none for a provider whose record has a token variable and no
  config folder variable. `NO_ACCOUNT` no longer covers that case. Nothing else of #256 changes.

What does not change: `docs/self-update.md`'s "nothing else installs, upgrades or restarts shed" (`install.*`
refuses shed's own binary), and #256's token files, which stay files.

## Consequences

- A release check, a VM lifecycle, a slot line and a sandbox wrap exist once. A new VM provider is a data file.
- shed gains about 2,250 lines and two crates, `shed-secrets` and `shed-hosts`; the cap needs one more raise.
- shed gains a second dependency decision after #256's `sha2`: `security-framework`, macOS only (question 2).
- The secrets file beside a lane's worktree is gone from every host, and with it `secrets-left` and its sweep.
- A lane's secret is in the environment of the run that named it and nowhere else on that machine.
- An unsigned shed, drover or cooee on macOS cannot save a secret, as an unsigned cooee cannot today. A locked
  Keychain is an error after 5 seconds, not a hang.
- cooee's secrets do not move: the crate opens the Keychain service and the file folder cooee uses now.
- The slot line is per machine and shared by every caller there: drover's lanes, its CI holds and anything
  cooee queues wait in one order. Limits set twice take the last values.
- A sandboxed agent cannot use the full shed CLI (no `run`, `exec`, `read`); the shed skill does not work
  inside a sandbox. Its hooks' state reports and its heavy commands do.
- ACP lanes can be sandboxed. Slice H5 starts with a live run on macOS and on Linux, which drover's spike never
  did for Linux.
- Rates are facts in the VM record; spend is still drover's sum over its own log. If drover stops, a running
  VM keeps running, as today: shed stops nothing by itself.
- `vm.*` runs on the main host only. A desk on another Mac is not in this plan.
- Recipes move from drover's config folder to shed's (`<config>/vms/`); drover's D6 moves an owner's override
  file once.
- Each shed slice that adds a method raises `PROTOCOL` by one (6 to 11), so an app's floor names the slice it
  needs. Nothing is dropped or retyped.

## Hot files

This work adds rows to the tables the hotspots plan (#246) turned into folders: methods, error codes, handlers,
commands, doctor rows. So each row is a new file (`crates/shed-proto/methods/vm_start.md`,
`crates/shed-proto/errors/unstaged.md`, `crates/shed-server/src/methods/vm_start.rs`,
`crates/shed/src/cmd/vm.rs`, `crates/shed/src/doctor/hosts.rs`), collected by #248 to #250's own lines. It
converts no existing hot file. What it adds is laid out so the six slices share no file:

- **Wire types, per area:** `crates/shed-proto/src/install.rs`, `vm.rs`, `slot.rs`, `sandbox.rs`, `secret.rs`.
- **Records:** one file per VM provider in `crates/shed-catalog/vms/`, collected by `build.rs` as `providers/` is.
- **The new crates:** one file per concern, listed in [README.md](README.md).
- **Docs, per area:** `docs/hosts/secrets.md`, `installs.md`, `slots.md`, `vms.md`, `machines.md`, `sandbox.md`.

The `.drover/arch.toml` lines. Slice H1 adds, and puts `"secrets"` into the `server` layer's `may_use`:

```toml
[[collect]]
dir = "docs/hosts"
glob = "*.md"

[[layer]]
name = "secrets"
paths = ["crates/shed-secrets/src/**"]
mods = ["shed_secrets"]
may_use = []

[[collect]]
dir = "crates/shed-secrets/src"
glob = "*.rs"
```

It also extends #256's `.expose()` ban: its `except` gains `"crates/shed-secrets/src/keychain.rs"` and
`"crates/shed-secrets/src/files.rs"`, and its `reason` gains "or a secret into its store".

Slice H2 adds, and puts `"hosts"` into the `server` layer's `may_use`:

```toml
[[layer]]
name = "hosts"
paths = ["crates/shed-hosts/src/**"]
mods = ["shed_hosts"]
may_use = ["core", "proto", "secrets"]

[[collect]]
dir = "crates/shed-hosts/src"
glob = "*.rs"
```

Slice H4 adds, and the `.expose()` ban's `except` gains `"crates/shed-hosts/src/vm.rs"`:

```toml
[[collect]]
dir = "crates/shed-catalog/vms"
glob = "*.toml"
```

Slice H5 adds:

```toml
[[ban]]
pattern = "inside.sock"
paths = ["crates/*/src/**"]
except = ["crates/shed-server/src/inside.rs"]
reason = "one module binds the inside socket and lists what it serves"
```
