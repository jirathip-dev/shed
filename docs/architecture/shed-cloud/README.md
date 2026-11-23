# shed-cloud: an optional helper binary that rents VMs (#307)

## Summary

Decision: `shed-cloud` is a new crate and binary in shed's repo that nothing else in shed depends on. It rents VMs from one data file per provider (Boat today) and runs by itself every 5 minutes on the main host. Lane hosts come from a warm pool: drover publishes how many lanes wait for which tags, and the helper creates disposable VMs from a golden snapshot within its own caps (VMs at once, VMs per tag, money per month), adds each as a shed machine with its address and pinned host key, and deletes it after a set number of lanes or an idle timeout. Before it ends a VM it marks the machine draining in shed: shed refuses new terminals there, and the VM goes only when no terminal is left. A VM you name yourself is started and stopped by hand, with its own monthly budget and idle stop. CI runner VMs use the same helper and are never shed machines. drover starts and stops nothing.
shed's server gets two small changes, neither about VMs: a machine can carry a pinned host key, and a shed can be draining. The link's fault tests use a stand-in for the ssh program, kept in shed's repo; the server gets no fault code.
Slices: (1) machines: a port, a pinned host key, an identity file; (2) draining; (3) the helper, by hand; (4) the 5-minute pass, its job and its release; (5) pools, demand and the per-VM git token; (6) the golden image; (7) runner groups; (8) the fake link, its fault tests and the soak script. Then drover's own issues (drover#1644, drover#1648). About 2,000 lines added in shed, about 850 leave drover.
Open questions: (1) the short-lived git token per VM needs a GitHub App you create once (one more secret on the main host; pushes and PRs from a VM then show the App as author): go with it (recommended), or keep today's long-lived token on VMs until you have one? (2) a VM you name yourself: only you start it and demand never does (recommended), or should demand also wake a stopped one, or do named VMs go once the pool works? (3) #259 put cooee's desks on VM methods in shed's server, which this ADR removes: desks stay in cooee as they are (recommended), or become a provider of the helper later? (4) one cap raise of about 1,600 lines in its own cap issue, sized by `drover cap berth` when filed.

The question, the options, the decision, what it reverses, its consequences and the `arch.toml` lines:
[adr.md](adr.md). Demand, sizing, `draining`, the golden image and the tokens: [pool.md](pool.md). The fake
link and the soak: [faults.md](faults.md). Config, record, log and wire shapes: [shapes.md](shapes.md). Where
each function of drover's `cloud.rs` and `runners.rs` goes: [port-map.md](port-map.md). This file has how the
callers read, the statecharts, the CLI, the module boundaries, the secrets, the migration and the issues to
file.

## How it reads

### the owner, in a terminal

```console
$ shed-cloud secret set BOAT_API_KEY          # the value from stdin, never an argument
$ shed-cloud install                          # the 5-minute job, dev.shed.cloud
$ shed-cloud image build dev                  # the first golden image; after that the pass rebuilds it
$ shed-cloud ls
NAME    KIND    STATE     SINCE  SPENT    MACHINE  NOTE
dev-17  pool    running   50m    $0.06    dev-17   lanes 1 of 2, 1 live
dev-16  pool    draining  12m    $0.21    dev-16   lanes 2 of 2; 1 terminal left
boat1   named   stopped   9h     $12/40   -        idle 20 min
bx_1    runner  running   6d     $10      -        app: 3/3 online

POOL  VMS  WAITING  IMAGE             NOTE
dev   2/3  0        dev-3f9a1c2b7d10  built 2 d ago
caps: 2 of 4 VMs, $31 of $150 this month
$ shed --on dev-17 ls                         # from here it is a machine like any other
$ shed-cloud drain dev-17                     # no new terminal; deleted when its terminals are gone
$ shed-cloud start boat1                      # a named VM is yours to start
$ shed-cloud stop boat1                       # drains it, then stops it
$ shed-cloud runner add app 2
$ shed machine drain --on box                 # shed's own: any machine, before a reboot
```

### drover, after drover#1575 (hosts are shed machines)

`[hosts.<n>]` keeps the persistent hosts with their tags and limits, and loses `cloud`, `size`,
`budget_usd_month`, `max_hours_day` and `idle_stop_minutes`. drover runs the helper as a command and reads JSON;
it links none of its code. It runs no `start`, `stop` or `rm`.

```rust
// once a pass: the rented machines; empty when the helper is not installed
let cloud: Listed = cloud::ls(r)?;                              // `shed-cloud ls --json`

// a host is a shed machine: the persistent ones of [hosts], and each running pool VM with its pool's tags and slots
let pool_vms = cloud.vms.iter().filter(|v| v.kind == Kind::Pool && v.state == State::Running);
let hosts: Vec<Host> = cfg.hosts().chain(pool_vms.map(|v| pool_host(v, &cloud.pools))).collect();

// placement: shed's stats say what a machine can take; a draining one takes nothing
let mut unmet = Demand::default();
for lane in waiting {
    let free = hosts.iter().filter(|h| h.covers(&lane.tags) && stats[&h.machine].draining.is_none() && h.has_room(&stats));
    match free.min_by_key(|h| h.cost()) {                       // persistent hosts first
        None => unmet.add(&lane.tags),
        Some(host) => match shed.run(lane.run_on(&host.machine)) {
            Err(e) if e.code == ErrorCode::Draining => unmet.add(&lane.tags),   // it began to drain: next pass
            started => lanes.record(started?),
        },
    }
}

// what still waits is published every pass, also when it is nothing
let plan: Plan = cloud::demand(r, "drover", &unmet)?;           // `shed-cloud demand --by drover --json`, the demand on stdin
board.waiting(&plan.short);                                     // "waits: gpu at its cap (1 of 1 VMs)"

// a lane ended on a draining machine: save its work, then remove its terminal; the removal lets the VM go
for lane in ended.iter().filter(|l| stats[&l.machine].draining.is_some()) {
    wip::save(r, lane)?;
    shed.rm(&lane.terminal)?;
}

// the Host cards and check's alerts read the same rows; drover keeps no cloud state
if vm.hold.is_some() { alert("cloud-hold", &vm.note) }          // drain_stuck, unreachable, key_changed, budget
if group.online < group.slots * group.vms { alert("runners-low", &group.note) }
```

### the helper's own pass

```rust
pub fn pass(cx: &Cx) -> Result<Vec<Note>> {
    let (config, log) = (Config::load(&cx.paths)?, Log::open(&cx.paths)?);
    for vm in life::known(&config, &log)? {                           // named VMs of the config; pool and image VMs of the log
        let Some(_held) = log.try_lock(&vm.name)? else { continue };  // a start or an end is at work on it
        let (lines, record) = (log.of(&vm.name)?, Record::load(&cx.paths, &vm.provider)?);
        let state = life::state(&lines);
        match (state, record.state(cx, &vm.name)?) {
            (State::Running | State::Draining, Seen::Running) => {}
            (State::Running | State::Draining, seen) => { drain::gone(seen).map(|proof| life::end(cx, &vm, proof)).transpose()?; continue }
            (State::Starting | State::Setup, _) => { life::resume_start(cx, &vm, &record)?; continue }
            _ => continue,
        }
        link::keep(cx, &vm, &record, &lines)?;                        // moved: machine.add replace; key changed: hold
        let stats = cx.shed.stats(&vm.machine)?;                      // None when it does not answer
        if state == State::Draining {
            if let Some(proof) = stats.and_then(|s| drain::zero(&s, drain::since(&lines))) {
                life::end(cx, &vm, proof)?;                           // machine.rm, then stop (named) or destroy
            }
            continue;
        }
        idle::note(cx, &vm, stats.as_ref(), &lines)?;                 // logs `quiet` or `busy`, and `lane` for a terminal seen first
        git::refresh(cx, &vm, &lines)?;                               // a token older than refresh_minutes
        if let Some(why) = limits::due(&config, &vm, &log.of(&vm.name)?, cx.now) {
            drain::mark(cx, &vm, why)?;                               // machine.drain; logs `drain`
        }
    }
    pool::size(cx, &config, &log)?;                                   // the fresh demand, `plan`, the creates: what `demand` runs too
    image::pass(cx, &config, &log)?;                                  // a key with no image: build
    config.runners.iter().try_for_each(|(name, group)| runner::pass(cx, name, group))   // tokens when due, health
}
```

`pool::plan`, `limits::due`, `spend::over`, `life::state`, `idle::quiet_since` and `drain::zero` take values
and touch nothing, so each rule is a table test.

## Statecharts

### A lane-host VM: named, pool or image

```
       start (a named VM, the owner)                              a live terminal
       demand (a pool VM, the helper)
 absent ─────────────▶ starting ─────▶ setup ─────▶ running ◀───────────────▶ running, quiet
   ▲                      │              │             │                            │
   │                      └──────┬───────┘             └─────────────┬──────────────┘
   │                      failed │                                   │ idle, lanes, age, budget, asked
   │                             ▼                                   ▼
   │       named ─────────▶   stopped  ◀── named, zero terminals ── draining
   │                             │ start                             │
   │                             ▼                                   │
   │                          starting                               │
   └──────── pool, image, or `rm`: zero terminals, then destroyed ───┘
```

| from | to | when | what the helper does |
|---|---|---|---|
| `absent` | `starting` | demand, for a pool VM; `start`, for a named one; and the budget allows it | verb `create_from` (a pool VM: the pool's image; a named VM once it has a snapshot), else `create`; logs `start` |
| `stopped` | `starting` | `start` on a named VM, and its budget allows it | verb `start`; logs `start` |
| `starting` | `setup` | the provider says `running`, within 5 minutes | logs `setup` |
| `setup` | `running` | every step passed and the link is up, within 2 minutes | `ready` (a new VM), `address`, the pin, `machine.add`, shed's build, a drain the disk carries lifted, the git token; logs `up` |
| `starting`, `setup` | `stopped` or `absent` | the wait ran out or a step failed | a named VM: verb `stop`; a pool VM: verb `destroy`; `machine.rm`; logs `stop` or `removed`, why `failed` |
| `running` | `running`, quiet | a pass sees no terminal outside `exited` since the last pass | logs `quiet` |
| `running`, quiet | `running` | a pass sees one | logs `busy` |
| `running` | `draining` | quiet for the idle limit; a pool VM's `max_lanes` or `max_life_hours`; the budget; `drain`, `stop` or `rm` | `machine.drain`; logs `drain` with why and the end it leads to |
| `draining` | `stopped` | a named VM, and the machine's stats list no terminal | `machine.rm`, verb `stop`; logs `stop` |
| `draining` | `absent` | a pool or image VM, or `rm`, and the machine's stats list no terminal | `machine.rm`, verb `destroy`; logs `removed` |
| `running`, `draining` | `stopped` or `absent` | a pass finds the provider does not say `running` | `machine.rm`; logs `stop` or `removed`, why `outside` |

| state | the log says | notes |
|---|---|---|
| `absent` | no line for it, or the last life line is `removed` | a pool VM's name is not used again |
| `starting` | `start` | spend runs from this line; a pass finishes or fails a start whose process died |
| `setup` | `setup` | the provider says running; the VM is not a machine yet |
| `running` | `up` | a machine; `quiet_since` is set while the last of `quiet` and `busy` after `up` is `quiet` |
| `draining` | `drain` | still a machine and still costing; its shed refuses `run` |
| `stopped` | `stop` | a named VM only; not a machine; spend ended at this line |

Four holds sit on top of a state and are shown in `ls`:

| hold | when | what lifts it |
|---|---|---|
| `budget` | a named VM's spend is at `budget_usd_month` or its hours at `max_hours_day`; the pools' spend is at `[caps] budget_usd_month`. `start` and every create refuse `OVER_BUDGET`, and what runs is drained | the month or day turning, or the owner's edit of `config.toml` |
| `key_changed` | `address` prints another key while the VM runs; the machine keeps the old pin, so its link fails closed | `shed-cloud repin <vm>`, or its end |
| `unreachable` | the provider says `running` but the machine's link has been down 15 minutes; a draining VM is not ended, since its terminals cannot be counted | the link coming back, or the owner's `--now` |
| `drain_stuck` | draining for 60 minutes with a terminal left | the terminal's removal, or the owner's `--now` |

Every command is safe to repeat: `start` on a running VM checks its address and exits 0; `drain` on a draining
one, `stop` on a stopped one and `rm` on an absent one change nothing.

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
| `healthy`, every `token_refresh_minutes` | a fresh token to every VM of the group, through the record's `token` line | the pass, `runner tokens` |
| `healthy` → `low` | fewer are online at a pass; the group's row shows it | the pass |
| `low` → `token` | a fresh token and a restart for that VM | `runner revive` |
| `draining` | the record's `drain` line; then no job on its runners, 30 minutes at most. A job still running after that: drained, not deleted, and `runner rm` again later | `runner rm` |
| `deregistered` → `gone` | each of its runners deleted at GitHub; verb `destroy`; `runner-rm` logged | `runner rm` |

A runner VM's `draining` is the runner service's own, on a VM with no shed: it is not shed's machine state.
`low` is reported, not repaired: the pass revives nothing by itself, as today. The pass also reads each
VM's `state`; one the provider no longer runs (its TTL passed) is logged `removed`, why `outside`, and stays in
the group's count until `runner rm`. `runner add` refuses `RUNNERS_CAP` past `max_vms` or `usd_day`; `runner rm`
of the last VM with online runners refuses `RUNNERS_LAST` without `--last`. A runner VM's spend is its rate
times the time from its `runner-add` line to its `removed` or `runner-rm` line.

## The CLI

Exit codes: 0 done, 1 an error (a verb failed: its stderr, with secrets cut out), 2 usage, 3 refused
(`REFUSED <CODE>: why` on stderr). Every command that prints takes `--json`.

| command | does | prints |
|---|---|---|
| `ls` | reads the log, the config and the demand files; asks no provider | one row per VM, pool and runner group, and the caps ([shapes.md](shapes.md)) |
| `demand --by <who>` | takes the lanes waiting, by tags, from stdin; stores it; sizes the pools; starts each create as its own process | what it created and what a cap held back (`Plan`); refuses `BAD_DEMAND` |
| `drain <vm> [--now]` | `machine.drain`; the pass ends the VM at zero terminals. `--now`, the owner's, ends it at once | `<vm> draining (<n> terminals)`, or how it ended |
| `start <vm>` | a named VM: create or resume, wait for running (5 min), set up, wait for the link (2 min); a failure stops it; on a running VM, only the address check | `<vm> running <user@host:port> (machine <m> up)`; refuses `OVER_BUDGET`, `KEY_CHANGED`, `NO_VM`, `NOT_NAMED`, `SHED_OLD` |
| `stop <vm> [--now]` | a named VM: drain, then at zero terminals verb `stop` and `machine.rm` | `<vm> draining (<n> terminals)` or `<vm> stopped`; refuses `NOT_NAMED` |
| `rm <vm> --yes [--now]` | drain, then at zero terminals verb `destroy` and `machine.rm` | `<vm> draining (<n> terminals)` or `<vm> removed` |
| `repin <vm>` | accepts the key `address` prints now; replaces the machine's entry | `<vm> repinned` and both key fingerprints |
| `snapshot <vm>` | a named VM: needs `running` and the link up; verb `snapshot`; waits for the link again | `<vm> snapshot saved` |
| `image build <pool>` / `image ls` | builds the pool's golden image now / lists each pool's images with key, age and state | each step as it passes; refuses `IMAGE_DIRTY` |
| `tick` | the pass | one line per change (`dev-16: removed (lanes 2 of 2)`); nothing when nothing changed |
| `log [--since T] [<vm>]` | the log's lines | `Line`s |
| `runner add <group> [n]` | create and set up n VMs | `<vm>: created (<size>), <slots> runners starting`; refuses `RUNNERS_CAP` |
| `runner rm <group> <vm> [--last]` | drain, deregister, destroy | `<vm>: drained, <n> runner(s) deregistered, deleted`; refuses `RUNNERS_LAST` |
| `runner revive <group> <vm>` | a fresh token and a service restart | `<vm>: fresh registration token, runner service restarted` |
| `runner tokens <group>` | a fresh token to every VM now | `tokens: a fresh one on <n>/<m> VMs` |
| `secret set <NAME>` / `secret ls` | saves stdin as a 0600 file / lists names and dates | never a value |
| `doctor` | rows: each record's key and identity file, its CLI's subcommands against `--help`, an override missing a key the built-in has, a pool with no image, `[git]` with no key, the job, shed's protocol here and on each running VM | `✓`, `!`, `✗` rows; exit 1 on a `✗` |
| `install` / `uninstall` | writes, loads or removes the job `dev.shed.cloud` | what it wrote |
| `import <config.toml> <log.jsonl>` | takes both files once, when its own log is empty | what it took |

shed's own CLI gains `shed machine add --host-key --identity --replace` (C1) and
`shed machine drain [--on M] [--off] [--why TEXT]` (C2).

## Module boundaries

```
  shed-cloud (binary) ──▶ shed-client ──▶ shed-proto     machine.add, machine.rm, machine.links, machine.stats,
        │        │                                       machine.drain, ls, run, rm, exec, hello
        │        └──────▶ shed-core                      runner (every process), clock
        ▼
  sh -c <a record's line>, gh, a pool's `key` command    all through shed_core::runner

  shed, shed-server and every other shed crate: no use of shed-cloud, no mention of it
  drover: runs `shed-cloud ls --json` and `shed-cloud demand`; links nothing of it; calls shed for the rest
  scripts/fakelink.py: stands where `ssh` stands, in tests only; no crate knows it
```

| file in `crates/shed-cloud/src/` | holds | may call |
|---|---|---|
| `main.rs`, `cmd/<verb>.rs` | arguments in, text or JSON out; one file per command | every module below |
| `out.rs` | the one stdout and stderr writer | nothing |
| `paths.rs` | the helper's config and state folders and shed's socket; the one reader of the environment | nothing |
| `config.rs` | `Config`, `Caps`, `Pool`, `Named`, `Group`, `Git`, the checked names | nothing |
| `record.rs` | a provider record and a git record: load, check, fill a line | `exec` |
| `exec.rs` | run a filled line: the key and a token on stdin, stderr with both cut out | core (`runner`) |
| `secret.rs` | `Secret`; the environment, else the 0600 file; `secret set` | `paths` |
| `log.rs` | `Line`, `Event`; append, read, the per-VM lock | `paths`, core (`clock`) |
| `life.rs` | `state` (pure); start, set up, `end` (which takes a `Drained`), snapshot; a start that died | `record`, `log`, `link`, `setup`, `spend`, `git` |
| `drain.rs` | `Drained` and its three makers; `mark` (`machine.drain`); `since` (pure) | client, `log` |
| `spend.rs` | spend, hours today, `over`; pure | nothing |
| `link.rs` | parse `address`; the pin; `machine.add` and `machine.rm`; wait for the link | client, `record`, `log` |
| `idle.rs` | `machine.stats` and `ls` into `quiet`, `busy` and `lane`; `quiet_since` (pure) | client, `log` |
| `limits.rs` | `due`: idle, lanes, age, budget; pure | `spend` |
| `demand.rs` | `Demand`: parse, store, the fresh ones | `paths`, core (`clock`) |
| `pool.rs` | `plan` (pure); `size`: the creates, each a detached start | `demand`, `life`, `log`, `spend` |
| `image.rs` | the key; the build; `never`; the newest two kept | `record`, `life`, `git`, client, core (`runner`, for the sums and the `key` command) |
| `git.rs` | mint on the main host, install and remove through shed's `exec`, `refresh` | `record`, `secret`, client, `log` |
| `setup.rs` | a VM's shed step: the release asset through the record's `exec` when the build differs | `record`, core (`runner`, for the sums) |
| `runner.rs` | a group: GitHub's list, a registration token, register, tokens, health, drain, deregister | `record`, `log`, core (`runner`, for `gh`) |
| `tick.rs` | the pass | `life`, `drain`, `link`, `idle`, `limits`, `pool`, `image`, `git`, `runner` |
| `service.rs` | the job, through farrier-service | farrier-service |

Rules, each with its enforcer:

- No shed crate uses `shed-cloud`, and it uses core, proto and client only (the `cloud` layer; the
  `shed-cloud` ban; the `cloud-free` gate on `cargo tree`).
- It names no provider: every provider fact is a record (the `boat` ban).
- It starts every process through `shed_core::runner` and reads the time through `shed_core::clock` (the
  existing bans, which cover `crates/*/src/**`).
- It reads the environment in `paths.rs` only (the `env::var` ban's one new exception). A test holds
  `paths::socket()` equal to `shed_server::Paths::user()`'s under the same variables.
- A VM ends only through `life::end`, which takes a `Drained` (the type: its field is private to `drain.rs`).
- `spend.rs` and `limits.rs` do no I/O, and `pool::plan`, `life::state`, `drain::zero` and `idle::quiet_since`
  take values and return one: table tests, with drover's cases from `crates/cli/tests/cloud.rs` and `runners.rs`.
- The tests drive the real binary against real shed servers in a temp folder, joined by the fake link, with a
  provider record whose verbs are shell lines over that folder ([faults.md](faults.md)).

In shed's server: C1 touches `machines.rs` (the entry's two new fields, one function that builds the ssh
options, used by `dial` and by the proxy forward in `proxy.rs`), `methods/machine_add.rs`, `shed-proto`'s
`machine.rs` and `shed`'s `cmd/machine.rs`. C2 adds `methods/machine_drain.rs`, one field and one check in the
registry's `run`, one field in `stats.rs`, the same two proto and CLI files. Nothing in either is about VMs.

## Secrets

| rule | held by |
|---|---|
| A secret is a `Secret`: no `Debug`, `Display`, `Serialize` or `Clone`. | the type; a compile-fail doc test |
| A provider's key and a git record's key are the variable the record names (`key_env`), else the 0600 file `secrets/<NAME>` in the helper's config folder. A file wider than 0600 is not read. | `SECRET_FILE_OPEN`; test; a doctor row with the `chmod` |
| `secret set` reads the value from stdin; `secret ls` gives names and dates. There is no get. | the CLI; test: every command's output after a set, searched for the value |
| A key reaches a line as a line on its shell's stdin, exported there under `key_env`. It is in no argument and in no environment the helper sets. | test: the runner's recorded arguments and environment |
| A registration token comes from `gh api` on the main host, under `gh`'s own login, and lives in one command's memory. It reaches the record's `exec` as a line on stdin, exported there as `$RT`, and no argument on the main host holds it. | test: the arguments and bytes of each process; the log and the output searched for the token |
| A git token comes from the git record's `mint` on the main host and lives in one command's memory. It reaches the VM as the stdin of shed's `exec`, over the pinned link, and is in no argument, no `exec` environment and no file on the main host. | test: the `exec` params the fake server saw; the log and the output searched for the token |
| No provider key, no App key and no agent login ever goes to a VM. An image is not saved while a `never` path exists. | the helper sends a VM only tokens; `IMAGE_DIRTY`; test with each `never` path present |
| A line's stderr is returned with the key and the token cut out. | test with a line that prints both |
| The log holds no secret: a host key is public, and no line has a field for a key or a token. | the `Event` type |
| Every value filled into a line matches its checked form first; `$C` and stdin are never filled in. | the checked name types; test with quotes, spaces and `$(…)` in each field |
| A verb that needs the VM's id and gets nothing does not run (boat reads an empty id as the VM it runs in). | `NO_VM`; test, moved from drover |
| `Secret::expose` is called in `exec.rs` only. | the `.expose()` ban's one new exception |
| No credential of ours goes to a runner VM, and no shed. | the runner path calls `exec` and `gh` only; test: the bytes of a `runner add` |

Not covered: the `boat` CLI's own login, which the helper never reads; and an unsandboxed lane on the main host,
which runs as the owner's user and can read a 0600 file (#259's security model says the same of shed's own).
A lane on a VM can read that VM's git token: it is what the lane pushes with, it reaches only `repos`, and it
dies within the hour.

## Migration

| step | repo | what | needs |
|---|---|---|---|
| 1 | shed | C1: a machine's port, pinned key and identity; `machine.add` replaces | nothing |
| 2 | shed | C2: draining | nothing |
| 3 | shed | C8: the fake link, the link's fault tests, the soak script | nothing (its two `--replace` rows: 1) |
| 4 | shed | C3: the helper, by hand: records, start, drain, stop, rm, repin, snapshot, spend, `ls`, `log`, `secret`, `doctor`, the checks | 1, 2, the cap issue |
| 5 | shed | C4: the pass, idle, the job, the release asset and self-update, `import` | 4 |
| 6 | shed | C5: pools, `demand`, lanes, the git token | 5 |
| 7 | shed | C6: the golden image | 6 |
| 8 | shed | C7: runner groups | 4 |
| 9 | drover | runners move (drover#1644): `drover runners` calls the helper or is deleted; `runners.rs` deleted | 8 |
| 10 | drover | lane hosts move (drover#1644): pool rows as hosts, `demand`, `draining`, save then remove, the image script and its key command, the export; `cloud.rs` deleted | 7, drover#1575 |
| 11 | drover | the multi-host harness and the nightly job that starts the soak (drover#1648) | 3 |

Steps 3, 4 and 8 can run side by side; `release.yml` is touched by steps 3 and 5, one after the other. Until step
10 the helper's config is empty and it does nothing, so two programs never end the same VM. Step 10 carries
the live state, in drover's own rollout:

1. The owner writes `[caps]`, a `[pool.<name>]` and `[git]`, sets the keys, and builds the first image.
2. drover writes the helper's two files from its own: `config.toml` from `[hosts.<n>]` and `[ci_runners]`, and
   `log.jsonl` from this month's `cloud-*` and every `runners-add` and `runners-remove` event
   ([shapes.md](shapes.md) has the key and event tables). drover owns that export: it knows its log.
3. `shed-cloud import` checks both with its own types and takes them. It refuses when its log is not empty.
4. The next pass finds each of today's VMs `running` in the log and at the provider, as a named VM, reads its
   address, and adds the machine with the key drover had pinned. No VM restarts, lanes on them run on, and this
   month's spend is the same number. They stop when idle; from then the pool takes new lanes, and the owner
   removes them with `rm` when they are no longer wanted.
5. drover's release from then on has no cloud pass. The owner removes the cloud keys from `fleet.toml`.

An owner's recipe override is moved by hand to the helper's `providers/` folder; `doctor` flags one that still
has drover's `runner_*` keys. The owner's fleet has no override and no `[ci_runners]` today (read 2026-11-23),
so step 9 carries no live state there.

## The issues to file

| # | title | scope | src lines |
|---|---|---|---|
| C1 | machines: a port, a pinned host key and an identity file | `ssh://user@host:port` targets tested; `Machine` and `MachineParams` gain `host_key` and `identity`, `MachineParams` gains `replace`; the config line's table form; one ssh-options function for `dial` and the proxy forward (`HostKeyAlias`, a known-hosts file per machine in the control folder, `StrictHostKeyChecking=yes`, `IdentitiesOnly=yes`); `shed machine add --host-key --identity --replace`; `PROTOCOL` + 1; `docs/commands/machine.md` | +90 |
| C2 | machines: draining | `machine.drain`; the state file; `run` refuses `DRAINING`; warm spares ended; `MachineStats.draining`; `shed machine drain`; `PROTOCOL` + 1; `docs/commands/machine.md` | +70 |
| C3 | shed-cloud 1: the helper, by hand | the crate, its layer, bans and gate; `providers/boat.toml` from drover's recipe; `record`, `exec`, `secret`, `log`, `life`, `drain`, `spend`, `link`, `setup`; `ls`, `start`, `drain`, `stop`, `rm`, `repin`, `snapshot`, `log`, `secret`, `doctor`; drover's cloud tests as table and binary tests; `docs/cloud/vms.md`, `providers.md`, `budget.md`, `machines.md` | +700 |
| C4 | shed-cloud 2: the pass, the job and the release | `tick`, `idle`, `limits`, a start that died; `install` and `uninstall`; `release.yml` builds, signs and publishes `shed-cloud-<target>`; `scripts/install.sh` takes it; `shed self-update` keeps the release's binaries installed beside `shed`; `import`; `docs/cloud/pass.md`; the README's Scope names the helper | +330 |
| C5 | shed-cloud 3: pools and demand | `[caps]`, `[pool]`; `demand`, `pool::plan` and `size`; lanes; a pool VM's limits; pool rows in `ls`; `git.rs` and `git/github-app.toml`; `docs/cloud/pool.md`, `tokens.md` | +330 |
| C6 | shed-cloud 4: the golden image | `image.rs`; `image build` and `image ls`; the key; `never`; the newest two kept; a bad image set aside; `docs/cloud/image.md` | +200 |
| C7 | shed-cloud 5: runner groups | `providers/boat-api.toml` with its `[runner]` lines; `runner.rs`; `runner add`, `rm`, `revive`, `tokens`; group rows in `ls`; drover's runner tests; `docs/cloud/runners.md` | +260 |
| C8 | links: the fake link, the fault tests and the soak | `scripts/fakelink.py` in place of the tests' fake `ssh`; `crates/shed/tests/machines/faults.rs`; the link fixes its table finds; `scripts/soak.sh`; the release asset `fakelink.py`; `docs/faults.md` | +0, plus the fixes |
| cap | cap: raise for shed-cloud | one raise, sized by `drover cap berth` on the day | 0 |

drover's side (delete `cloud.rs` and `runners.rs`, the export, pool rows as hosts, `demand`, the save and
remove on a draining machine, the image script, its harness and nightly job, its docs) is drover#1644's and
drover#1648's to slice; [port-map.md](port-map.md) lists what stays there.
