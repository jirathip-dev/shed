# shed-cloud: an optional helper binary that rents VMs (#307)

## Summary

Decision: `shed-cloud` is a new crate and binary in shed's repo that nothing else in shed depends on. It rents VMs from one data file per provider (Boat today), keeps each VM's monthly budget, and by itself, every 5 minutes on the main host, stops a VM that is over budget or has had no live terminal for its idle limit. A lane-host VM is a shed machine exactly while it runs: the helper adds it with its address and pinned host key at each start and removes it at each stop. A CI runner VM is never a shed machine; its setup step registers GitHub runners. drover calls two commands, list and start, and otherwise sees only shed machines.
Slices: (1) shed's machine list takes a port, a pinned host key and an identity file; (2) the helper: provider records, start, stop, remove, snapshot, spend and budget, list, doctor; (3) the 5-minute pass, idle stop, its job, its release and the import from drover; (4) runner pools. Then drover's own issues (drover#1644). About 1,400 lines added in shed, about 850 leave drover.
Open questions: (1) #259 put cooee's desks on VM methods in shed's server, which this ADR removes: desks stay in cooee as they are now (recommended), or become a second provider of the helper later? (2) one cap raise of about 1,000 lines in its own cap issue, sized by `drover cap berth` when filed.

The question, the options, the decision, what it reverses, its consequences and the `arch.toml` lines:
[adr.md](adr.md). Config, provider record, log and wire shapes: [shapes.md](shapes.md). Where each function of
drover's `cloud.rs` and `runners.rs` goes: [port-map.md](port-map.md). This file has how the callers read, the
statecharts, the CLI, the module boundaries, the secrets, the migration and the issues to file.

## How it reads

### the owner, in a terminal

```console
$ shed-cloud secret set BOAT_API_KEY          # the value from stdin, never an argument
$ shed-cloud install                          # the 5-minute job, dev.shed.cloud
$ shed-cloud start boat1
boat1 running user@vm.example:2222 (machine boat1 up)
$ shed-cloud ls
NAME   KIND    STATE    SINCE  SPENT    TODAY   MACHINE  NOTE
boat1  host    running  2h     $12/40   3.1h/6  boat1    quiet 4 of 20 min
boat2  host    stopped  9h     $40/40   0.0h    -        over budget: spent $40.02 of $40 this month
bx_1   runner  running  6d     $10      24.0h   -        app: 3/3 online
$ shed --on boat1 ls                          # from here it is a machine like any other
$ shed-cloud runner add app 2
$ shed-cloud stop boat1
$ shed-cloud rm boat2 --yes                   # destroys the VM; the owner's
```

### drover, after drover#1575 (hosts are shed machines)

`[hosts.boat1]` keeps `machine = "boat1"` and its tags and limits, and loses `cloud`, `size`,
`budget_usd_month`, `max_hours_day` and `idle_stop_minutes`. drover runs the helper as a command and reads JSON;
it links none of its code.

```rust
// survey, once a pass: which machines are rented, and what holds each
let vms: BTreeMap<String, VmRow> = cloud::ls(r)?;          // `shed-cloud ls --json`; empty when it is not installed

// placement: a host whose machine has no link
match vms.get(machine) {
    None => offline(host),                                 // an ordinary machine: #1575's down_since rule
    Some(vm) if vm.hold == Some(Hold::Budget) => refuse("CLOUD_BUDGET", &vm.note),
    Some(vm) if vm.state == State::Stopped => startable(host, "CLOUD_STOPPED"),
    Some(_) => wait(host),                                 // starting or setup: another caller is at it
}

// wake: placement picked a stopped one (never on a dry run)
cloud::start(r, &vm.name)?;                                // `shed-cloud start boat1`: 0 once its link is up, 3 `OVER_BUDGET`
place_again(host);                                         // shed.stats(machine), shed.run(RunParams { machine, .. })

// the Host card and check's alerts read the same rows; drover keeps no cloud state
card(vm.spent_usd_month, vm.budget_usd_month, vm.hours_today, vm.since);
if vm.hold == Some(Hold::StopAborted) { alert("cloud-abort", &vm.note) }
if pool.online < pool.slots * pool.vms { alert("runners-low", &pool.note) }
```

Its wip save runs as the VM's hook, set once in the helper's config:

```toml
[vm.boat1]
before_stop = ["drover", "host", "save", "boat1"]   # drover's command; exit 0 or the stop is aborted
```

### the helper's own pass

```rust
pub fn pass(cx: &Cx) -> Result<Vec<Note>> {
    let (config, log) = (Config::load(&cx.paths)?, Log::open(&cx.paths)?);
    for (name, vm) in &config.vm {
        let Some(_held) = log.try_lock(name)? else { continue };      // a start or stop is at work on it
        let lines = log.of(name)?;
        let record = Record::load(&cx.paths, &vm.provider)?;
        match (life::state(&lines), record.state(cx, name)?) {
            (State::Running, Seen::Running) => {}
            (State::Running, seen) => { life::stopped(cx, name, Why::Outside, seen)?; continue }
            (State::Starting | State::Setup, _) => { life::resume_start(cx, name, vm, &record)?; continue }
            _ => continue,
        }
        link::keep(cx, name, vm, &record, &lines)?;                    // moved: machine.add replace; key changed: hold
        let idle = idle::look(cx, name, vm, &lines)?;                  // machine.stats; logs `quiet` or `busy`; Some past the limit
        if let Some(why) = spend::over(vm, &record, &lines, cx.now).or(idle) {
            life::stop(cx, name, vm, &record, why)?;                   // before_stop, the verb, machine.rm, `stop`
        }
    }
    config.runners.iter().try_for_each(|(name, pool)| runner::pass(cx, name, pool))   // tokens when due, health
}
```

`spend::over`, `life::state` and `idle::quiet_since` take the log's lines and a time and touch nothing, so each
rule is a table test.

## Statecharts

### A lane-host VM

```
            start                                                a live terminal
 absent ───────────▶ starting ───────▶ setup ───────▶ running ◀─────────────────▶ running, quiet
   ▲                    │                │               │      no live terminal        │
   │ rm --yes           └────────────────┴───────┬───────┴──────────────────────────────┘
   │                                             ▼
   └───────────────────────────────────────── stopped ────start────▶ starting
```

| from | to | when | what the helper does |
|---|---|---|---|
| `absent`, `stopped` | `starting` | `start`, and the budget allows it | verb `create` (`create_from` once snapshotted) when the provider has no such VM, else `start`; logs `start` |
| `starting` | `setup` | the provider says `running`, within 5 minutes | logs `setup` |
| `setup` | `running` | every step passed and the link is up, within 2 minutes | `ready` (a new VM), `address`, the pin, `machine.add`, shed on the VM; logs `up` |
| `starting`, `setup` | `stopped` | the wait ran out or a step failed | verb `stop`, `machine.rm`; logs `stop`, why `failed` |
| `running` | `running`, quiet | a pass sees no terminal outside `exited` since the last pass | logs `quiet` |
| `running`, quiet | `running` | a pass sees one | logs `busy` |
| `running` | `stopped` | quiet for `idle_stop_minutes`; or spend at the budget; or `stop` | `before_stop`, verb `stop`, `machine.rm`; logs `stop`, why `idle`, `budget` or `asked` |
| `running` | `stopped` | a pass finds the provider does not say `running` | `machine.rm`; logs `stop`, why `outside` |
| any | `absent` | `rm --yes` | `before_stop` if running, verb `destroy`, `machine.rm`; logs `removed` |

| state | the log says | notes |
|---|---|---|
| `absent` | no line for it, or the last life line is `removed` | a `start` creates it |
| `starting` | `start` | spend runs from this line; a pass finishes or fails a start whose process died |
| `setup` | `setup` | the provider says running; the VM is not a machine yet |
| `running` | `up` | a machine; `quiet_since` is set while the last of `quiet` and `busy` after `up` is `quiet` |
| `stopped` | `stop` | not a machine; spend ended at this line |

Three holds sit on top of a state and are shown in `ls`:

| hold | when | what lifts it |
|---|---|---|
| `budget` | spend this month is at `budget_usd_month`, or hours today at `max_hours_day`; `start` refuses `OVER_BUDGET` and a running VM is stopped at the next pass | the month or day turning, or the owner's edit of `config.toml` |
| `key_changed` | `address` prints another key while the VM runs; the machine keeps the old pin, so its link fails closed | `shed-cloud repin <vm>`, or a stop and start |
| `stop_aborted` | `before_stop` exited non-zero; the VM still runs and each pass tries again | the hook passing, or `shed-cloud stop <vm> --no-hook` (the owner's) |

Every command is safe to repeat: `start` on a running VM checks its address and exits 0, `stop` on a stopped one
and `rm` on an absent one change nothing.

### A runner VM and its setup step

```
 runner add ──▶ created ──▶ token ──▶ register ──▶ waiting ──▶ healthy ◀──▶ low
                              ▲                                   │          │
                              └────────── runner revive ──────────┴──────────┘

 healthy, low ──runner rm──▶ draining ──▶ deregistered ──▶ gone
```

| step | what happens | by |
|---|---|---|
| `created` | verb `create`; the VM's runner-name prefix is read once (the record's `names` line) and logged with `runner-add` | `runner add` |
| `token` | `gh api` on the main host asks GitHub for the repo's registration token; it stays in memory | `runner add`, `runner revive` |
| `register` | the record's `revive` line through `exec`: the token on stdin, the runner service restarted | the same command |
| `waiting` → `healthy` | GitHub lists `slots` runners of that prefix online | the pass |
| `healthy`, every `token_refresh_minutes` | a fresh token to every VM of the pool, through the record's `token` line | the pass, `runner tokens` |
| `healthy` → `low` | fewer are online at a pass; the pool row shows it | the pass |
| `low` → `token` | a fresh token and a restart for that VM | `runner revive` |
| `draining` | the record's `drain` line; then no job on its runners, 30 minutes at most. A job still running after that: drained, not deleted, and `runner rm` again later | `runner rm` |
| `deregistered` → `gone` | each of its runners deleted at GitHub; verb `destroy`; `runner-rm` logged | `runner rm` |

`low` is reported, not repaired: the pass revives nothing by itself, as today. The pass also reads each pool
VM's `state`; one the provider no longer runs (its TTL passed) is logged `stop`, why `outside`, and stays in the
pool's count until `runner rm`. `runner add` refuses `RUNNERS_CAP` past `max_vms` or `usd_day`; `runner rm` of
the last VM with online runners refuses `RUNNERS_LAST` without `--last`. A runner VM's spend is its rate times
the time from its `runner-add` line to its `stop` or `runner-rm` line.

## The CLI

Exit codes: 0 done, 1 an error (a verb failed: its stderr, with secrets cut out), 2 usage, 3 refused
(`REFUSED <CODE>: why` on stderr). Every command that prints takes `--json`.

| command | does | prints |
|---|---|---|
| `ls` | reads the log and config; asks no provider | one row per VM (`VmRow`), one per pool |
| `start <vm>` | create or resume, wait for running (5 min), set up, wait for the link (2 min); a failure stops the VM; on a running VM, only the address check | `<vm> running <user@host:port> (machine <m> up)`; refuses `OVER_BUDGET`, `KEY_CHANGED`, `NO_VM`, `SHED_OLD` |
| `stop <vm> [--no-hook]` | `before_stop`, the verb, `machine.rm` | `<vm> stopped (<why>)`; refuses `STOP_ABORTED` |
| `rm <vm> --yes` | `before_stop` if running, verb `destroy`, `machine.rm` | `<vm> removed` |
| `repin <vm>` | accepts the key `address` prints now; replaces the machine's entry | `<vm> repinned` and both key fingerprints |
| `snapshot <vm>` | needs `running` and the link up; verb `snapshot`; waits for the link again | `<vm> snapshot saved` |
| `tick` | the pass | one line per change (`boat1: stopped (idle 20 min)`); nothing when nothing changed |
| `log [--since T] [<vm>]` | the log's lines | `Line`s |
| `runner add <pool> [n]` | create and set up n VMs | `<vm>: created (<size>), <slots> runners starting`; refuses `RUNNERS_CAP` |
| `runner rm <pool> <vm> [--last]` | drain, deregister, destroy | `<vm>: drained, <n> runner(s) deregistered, deleted`; refuses `RUNNERS_LAST` |
| `runner revive <pool> <vm>` | a fresh token and a service restart | `<vm>: fresh registration token, runner service restarted` |
| `runner tokens <pool>` | a fresh token to every VM now | `tokens: a fresh one on <n>/<m> VMs` |
| `secret set <NAME>` / `secret ls` | saves stdin as a 0600 file / lists names and dates | never a value |
| `doctor` | rows: each record's key and identity file, its CLI's subcommands against `--help`, an override missing a key the built-in has, the job, shed's protocol, each running VM's machine | `✓`, `!`, `✗` rows; exit 1 on a `✗` |
| `install` / `uninstall` | writes, loads or removes the job `dev.shed.cloud` | what it wrote |
| `import <config.toml> <log.jsonl>` | takes both files once, when its own log is empty | what it took |

## Module boundaries

```
  shed-cloud (binary) ──▶ shed-client ──▶ shed-proto          machine.add, machine.rm, machine.links, machine.stats, hello
        │        └──────▶ shed-core                           runner (every process), clock
        ▼
  sh -c <a record's verb>, gh, curl, a before_stop command    all through shed_core::runner

  shed, shed-server and every other shed crate: no use of shed-cloud, no mention of it
  drover: runs `shed-cloud ls --json` and `shed-cloud start`; links nothing of it
```

| file in `crates/shed-cloud/src/` | holds | may call |
|---|---|---|
| `main.rs`, `cmd/<verb>.rs` | arguments in, text or JSON out; one file per command | every module below |
| `out.rs` | the one stdout and stderr writer | nothing |
| `paths.rs` | the helper's config and state folders and shed's socket; the one reader of the environment | nothing |
| `config.rs` | `Config`, `Host`, `Pool`, the checked names | nothing |
| `record.rs` | a provider record: load, check, fill a verb | `exec` |
| `exec.rs` | run a filled verb: the key and a token on stdin, stderr with both cut out | core (`runner`) |
| `secret.rs` | `Secret`; the environment, else the 0600 file; `secret set` | `paths` |
| `log.rs` | `Line`, `Event`; append, read, the per-VM lock | `paths`, core (`clock`) |
| `life.rs` | `state` (pure); start, stop, remove, snapshot; a start that died | `record`, `log`, `link`, `setup`, `spend` |
| `spend.rs` | spend, hours today, `over`; pure | nothing |
| `link.rs` | parse `address`; the pin; `machine.add` and `machine.rm`; wait for the link | client, `record`, `log` |
| `idle.rs` | `machine.stats` into `quiet` and `busy`; `quiet_since` (pure) | client, `log` |
| `setup.rs` | a host's one setup step: shed on the VM, through the record's `exec` | `record`, core (`runner`, for the sums) |
| `runner.rs` | a pool: GitHub's list, a registration token, register, tokens, health, drain, deregister | `record`, `log`, core (`runner`, for `gh`) |
| `tick.rs` | the pass | `life`, `link`, `idle`, `spend`, `runner` |
| `service.rs` | the job, through farrier-service | farrier-service |

Rules, each with its enforcer:

- No shed crate uses `shed-cloud`, and it uses core, proto and client only (the `cloud` layer; the
  `shed-cloud` ban; the `cloud-free` gate on `cargo tree`).
- It names no provider: every provider fact is a record (the `boat` ban).
- It starts every process through `shed_core::runner` and reads the time through `shed_core::clock` (the
  existing bans, which cover `crates/*/src/**`).
- It reads the environment in `paths.rs` only (the `env::var` ban's one new exception). A test holds
  `paths::socket()` equal to `shed_server::Paths::user()`'s under the same variables.
- `spend.rs` does no I/O, and `life::state` and `idle::quiet_since` take lines and return a value: table tests,
  with drover's cases from `crates/cli/tests/cloud.rs` and `runners.rs`.
- The tests drive the real binary against a record whose verbs are shell lines over a temp folder and
  `shed-client`'s fake server.

In shed's server, slice 1 only: `machines.rs` (the entry's two new fields, one function that builds the ssh
options, used by `dial` and by the proxy forward in `proxy.rs`), `methods/machine_add.rs`, `shed-proto`'s
`machine.rs` and `shed`'s `cmd/machine.rs`. Nothing there is about VMs: any machine may be pinned.

## Secrets

| rule | held by |
|---|---|
| A secret is a `Secret`: no `Debug`, `Display`, `Serialize` or `Clone`. | the type; a compile-fail doc test |
| A provider's key is the variable its record names (`key_env`), else the 0600 file `secrets/<NAME>` in the helper's config folder. A file wider than 0600 is not read. | `SECRET_FILE_OPEN`; test; a doctor row with the `chmod` |
| `secret set` reads the value from stdin; `secret ls` gives names and dates. There is no get. | the CLI; test: every command's output after a set, searched for the value |
| A key reaches a verb as a line on its shell's stdin, exported there under `key_env`. It is in no argument and in no environment the helper sets. | test: the runner's recorded arguments and environment |
| A registration token comes from `gh api` on the main host, under `gh`'s own login, and lives in one command's memory. It reaches the record's `exec` as a line on stdin, exported there as `$RT`, and no argument on the main host holds it. | test: the arguments and bytes of each process; the log and the output searched for the token |
| A verb's stderr is returned with the key and the token cut out. | test with a verb that prints both |
| The log holds no secret: a host key is public, and no line has a field for a key or a token. | the `Event` type |
| Every value filled into a verb matches `[A-Za-z0-9_-]+` first; `$C` and stdin are never filled in. | the checked name types; test with quotes, spaces and `$(…)` in each field |
| A verb that needs the VM's id and gets nothing does not run (boat reads an empty id as the VM it runs in). | `NO_VM`; test, moved from drover |
| `Secret::expose` is called in `exec.rs` only. | the `.expose()` ban's one new exception |
| No credential of ours goes to a runner VM, and no shed. | the runner path calls `exec` and `gh` only; test: the bytes of a `runner add` |

Not covered: the `boat` CLI's own login, which the helper never reads; and an unsandboxed lane on the main host,
which runs as the owner's user and can read a 0600 file (#259's security model says the same of shed's own).

## Migration

| step | repo | what | needs |
|---|---|---|---|
| 1 | shed | C1: a machine's port, pinned key and identity; `machine.add` replaces | nothing |
| 2 | shed | C2: the helper, by hand: records, start, stop, rm, repin, snapshot, spend, `ls`, `log`, `secret`, `doctor`, the checks | 1, the cap issue |
| 3 | shed | C3: the pass, idle stop, `before_stop`, the job, the release asset and self-update, `import` | 2 |
| 4 | shed | C4: runner pools | 2 |
| 5 | drover | runners move (drover#1644): `drover runners` calls the helper or is deleted; `runners.rs` deleted | 4 |
| 6 | drover | lane hosts move (drover#1644): export, `shed-cloud import`, placement on `ls` and `start`, the save command; `cloud.rs` deleted | 3, drover#1575 |

Steps 3 and 4 share no file and can run side by side. Until step 6 the helper's config is empty and it does
nothing, so two programs never stop the same VM. Step 6 carries the live state, in drover's own rollout:

1. drover writes the helper's two files from its own: `config.toml` from `[hosts.<n>]` and `[ci_runners]`, and
   `log.jsonl` from this month's `cloud-*` and every `runners-add` and `runners-remove` event
   ([shapes.md](shapes.md) has the key and event tables). drover owns that export: it knows its log.
2. `shed-cloud import` checks both with its own types and takes them. It refuses when its log is not empty.
3. The next pass finds each VM `running` in the log and at the provider, reads its address, and adds the machine
   with the key drover had pinned. No VM restarts, and this month's spend is the same number.
4. drover's release from then on has no cloud pass. The owner removes the cloud keys from `fleet.toml`.

An owner's recipe override is moved by hand to the helper's `providers/` folder; `doctor` flags one that still
has drover's `runner_*` keys. The owner's fleet has no override and no `[ci_runners]` today (read 2026-11-23),
so step 5 carries no live state there.

## The issues to file

| # | title | scope | src lines |
|---|---|---|---|
| C1 | machines: a port, a pinned host key and an identity file | `ssh://user@host:port` targets tested; `Machine` and `MachineParams` gain `host_key` and `identity`, `MachineParams` gains `replace`; the config line's table form; one ssh-options function for `dial` and the proxy forward (`HostKeyAlias`, a known-hosts file per machine in the control folder, `StrictHostKeyChecking=yes`, `IdentitiesOnly=yes`); `shed machine add --host-key --identity --replace`; `PROTOCOL` + 1; `docs/commands/machine.md` | +90 |
| C2 | shed-cloud 1: the helper, by hand | the crate, its layer, bans and gate; `providers/boat.toml` from drover's recipe; `record`, `exec`, `secret`, `log`, `life`, `spend`, `link`, `setup`; `ls`, `start`, `stop`, `rm`, `repin`, `snapshot`, `log`, `secret`, `doctor`; drover's cloud tests as table and binary tests; `docs/cloud/vms.md`, `providers.md`, `budget.md`, `machines.md` | +700 |
| C3 | shed-cloud 2: the pass, the job and the release | `tick`, `idle`, `before_stop`, a start that died; `install` and `uninstall`; `release.yml` builds, signs and publishes `shed-cloud-<target>`; `scripts/install.sh` takes it; `shed self-update` keeps the release's binaries installed beside `shed`; `import`; `docs/cloud/pass.md`; the README's Scope names the helper | +330 |
| C4 | shed-cloud 3: runner pools | `providers/boat-api.toml` with its `[runner]` lines; `runner.rs`; `runner add`, `rm`, `revive`, `tokens`; pool rows in `ls`; drover's runner tests; `docs/cloud/runners.md` | +260 |
| cap | cap: raise for shed-cloud | one raise, sized by `drover cap berth` on the day | 0 |

drover's side (delete `cloud.rs` and `runners.rs`, the export, placement on `ls` and `start`, the `host save`
command, its docs) is drover#1644's ADR to slice; [port-map.md](port-map.md) lists what stays there.
