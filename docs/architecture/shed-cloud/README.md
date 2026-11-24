# shed-cloud: an optional helper binary that rents VMs (#307)

## Summary

Decision: `shed-cloud` is a new crate and binary in shed's repo that nothing else in shed depends on. It rents VMs from one data file per provider (Boat today) and runs by itself every 5 minutes on the main host. Lane hosts come from a warm pool: drover publishes how many lanes wait for which tags, and the helper creates disposable VMs from a golden snapshot within its own caps (VMs at once, VMs per tag, money per month) and adds each as a shed machine with its address and pinned host key. A pool VM takes 2 issues by default, counted by the `item` label on its terminals, so one issue's impl, review and fix rounds stay on one warm VM; it is deleted when their terminals are gone, when it is idle, or at its age limit. Before it ends a VM the helper asks shed to drain the machine, and the VM goes only when shed lists no terminal on it. A VM you name yourself is started by you only, with its own monthly budget and idle stop. CI runner VMs use the same helper and are never shed machines. drover starts and stops nothing.
The helper keeps no machine state of its own: draining, key changed, added, up, offline and removed are rows of shed's transition table (#309); the helper reads them as facts and asks shed for the actions. Its tests run over #308's fake link, and its idle, budget and age rules take the time as a value (#310), so no test waits on a real clock.
Slices: (1) machines: a port, a pinned host key, an identity file; (2) draining; (3) the helper, by hand; (4) the 5-minute pass, its job and its release; (5) pools, demand, the per-VM git token, the pool VM in the nightly soak; (6) the golden image; (7) runner groups. Slices 1 and 2 need #309's slices 1 and 2; slices 3 to 7 also need #309's terminal and machine take-overs. Then drover's own issues (drover#1644, drover#1648). About 2,000 lines added in shed, about 850 leave drover.
Open questions: none. The owner's answers of 2026-11-23 are decisions here: a GitHub App mints each VM's git token, a named VM is started by the owner only, cooee's desks stay in cooee, and the cap raise is its own issue, filed when slice 3 starts.

The question, the options, the decision, what it reverses, its consequences and the `arch.toml` lines:
[adr.md](adr.md). Demand, sizing, `draining`, the golden image and the tokens: [pool.md](pool.md). Config,
record, log and wire shapes: [shapes.md](shapes.md). Where each function of drover's `cloud.rs` and
`runners.rs` goes: [port-map.md](port-map.md). This file has how the callers read, the statecharts and the rows
of #309's table behind them, the CLI, the module boundaries, the secrets, the tests, the migration and the
issues to file.

Read on shed staging `c1fba0b`, which has #308's fake link and #309's accepted ADR
([../state-table/](../state-table/README.md)).

## How it reads

### the owner, in a terminal

```console
$ shed-cloud secret set BOAT_API_KEY          # the value from stdin, never an argument
$ shed-cloud install                          # the 5-minute job, dev.shed.cloud
$ shed-cloud image build dev                  # the first golden image; after that the pass rebuilds it
$ shed-cloud ls
NAME    KIND    VM       SINCE  SPENT    MACHINE  NOTE
dev-17  pool    running  50m    $0.06    dev-17   items 1 of 2: shed#307
dev-16  pool    running  2h     $0.21    dev-16   items 2 of 2: app#12, app#15; leaving 12m: idle 20 min
boat1   named   stopped  9h     $12/40   -        idle 20 min
bx_1    runner  running  6d     $10      -        app: 3/3 online

POOL  VMS  WAITING  IMAGE             NOTE
dev   2/3  0        dev-3f9a1c2b7d10  built 2 d ago
caps: 2 of 4 VMs, $31 of $150 this month
$ shed machine ls                             # the machine's own state is shed's to show (#309)
NAME    TARGET              MACHINE   LINK  SINCE  NOTE
dev-17  ssh://dev@…:2201    up        up    50m
dev-16  ssh://dev@…:2207    draining  up    12m    idle 20 min
$ shed --on dev-17 ls                         # from here it is a machine like any other
$ shed-cloud drain dev-17                     # it leaves: no new terminal; deleted when its terminals are gone
$ shed-cloud start boat1                      # a named VM is yours to start; demand never does
$ shed-cloud stop boat1                       # drains it, then stops it
$ shed-cloud runner add app 2
$ shed machine drain --on box                 # shed's own: any machine, before a reboot
```

### drover, after drover#1575 (hosts are shed machines)

`[hosts.<n>]` keeps the persistent hosts with their tags and limits, and loses `cloud`, `size`,
`budget_usd_month`, `max_hours_day` and `idle_stop_minutes`. drover runs the helper as a command and reads JSON;
it links none of its code. It runs no `start`, `stop` or `rm`.

```rust
// once a pass: the rented VMs (empty when the helper is not installed) and every machine's state, from shed
let cloud: Listed = cloud::ls(r)?;                              // `shed-cloud ls --json`
let machines = shed.machines()?;                                // `machine.ls`: each machine's `flow` (#309)

// a host is a shed machine: the persistent ones of [hosts], and each running pool VM that is not leaving
let pool_vms = cloud.vms.iter().filter(|v| v.kind == Kind::Pool && v.state == Vm::Running && v.leaving.is_none());
let hosts: Vec<Host> = cfg.hosts().chain(pool_vms.map(|v| pool_host(v, &cloud.pools))).collect();

let mut unmet = Demand::default();
for lane in waiting {
    let free = hosts.iter().filter(|h| {
        h.covers(&lane.tags)
            && machines[&h.machine].flow.state == Machine::Up   // draining, offline, key_changed: nothing new there
            && h.takes(&lane.item, &lanes)                      // a pool VM: an issue it has had, or a new one under `max_lanes`
            && h.has_room(&stats)
    });
    match free.min_by_key(|h| (!h.has(&lane.item), h.cost())) { // the VM that is warm for this issue, then the cheapest
        None => unmet.add(&lane.tags),
        Some(host) => match shed.run(&lane.run_on(&host.machine)) {   // its labels carry `group` and `item`, as spawn sets them today
            Err(e) if e.is(ErrorCode::Draining) => unmet.add(&lane.tags),   // it began to drain: next pass
            started => lanes.record(started?),
        },
    }
}

// what still waits is published every pass, also when it is nothing
let plan: Plan = cloud::demand(r, "drover", &unmet)?;           // `shed-cloud demand --by drover --json`, the demand on stdin
board.waiting(&plan.short);                                     // "waits: gpu at its cap (1 of 1 VMs)"

// a lane ended on a draining machine: save its work, then remove its terminal; the removal lets the VM go
for lane in ended.iter().filter(|l| machines[&l.machine].flow.state == Machine::Draining) {
    wip::save(r, lane)?;
    shed.rm(&lane.terminal)?;
}

// alerts: a machine's trouble is shed's `stuck` event or its `flow`, for any machine; only money is the helper's
if let Some(why) = &vm.over { alert("cloud-budget", why) }
if group.online < group.slots * group.vms { alert("runners-low", &group.note) }
```

An item is a terminal's `group` and `item` labels together (`shed#307`), so two repos' issue 12 are two.
`h.takes` counts the row's `items` and the items of drover's own lanes on that machine, which drover knows
before the helper's next pass does.

### the helper's own pass

```rust
pub fn pass(cx: &Cx) -> Result<Vec<Note>> {
    let (config, log) = (Config::load(&cx.paths)?, Log::open(&cx.paths)?);
    for vm in life::known(&config, &log)? {                           // named VMs of the config; pool and image VMs of the log
        let Some(_held) = log.try_lock(&vm.name)? else { continue };  // a start or an end is at work on it
        let (lines, record) = (log.of(&vm.name)?, Record::load(&cx.paths, &vm.provider)?);
        match (life::state(&lines), record.state(cx, &vm.name)?) {
            (State::Running, Seen::Running) => {}
            (State::Running, seen) => { leave::gone(seen).map(|proof| life::end(cx, &vm, proof)).transpose()?; continue }
            (State::Starting | State::Setup, _) => { life::resume_start(cx, &vm, &record)?; continue }
            _ => continue,
        }
        address::keep(cx, &vm, &record, &lines)?;                     // the provider names another address: `machine.add` again (row M8)
        let Some(seen) = cx.shed.counts(&vm.machine)? else { continue };   // shed has no answer from it: nothing is decided
        if let Some(leaving) = life::leaving(&lines) {
            match leave::zero(&seen) {
                Some(proof) => life::end(cx, &vm, proof)?,            // `machine.rm`, then stop (named) or destroy
                None if !seen.draining => cx.shed.drain(&vm.machine, &leaving.why)?,   // lifted by hand, or the first ask was lost
                None => {}
            }
            continue;
        }
        idle::note(cx, &vm, &seen, &lines)?;                          // logs `quiet` or `busy`, and `item` for a label value seen first
        git::refresh(cx, &vm, &lines)?;                               // a token older than refresh_minutes
        if let Some(why) = limits::due(&config, &vm, &seen, &log.of(&vm.name)?, cx.now) {
            leave::decide(cx, &vm, why)?;                             // logs `leave`; asks shed: `machine.drain`
        }
    }
    pool::size(cx, &config, &log)?;                                   // the fresh demand, `plan`, the creates: what `demand` runs too
    image::pass(cx, &config, &log)?;                                  // a key with no image: build
    config.runners.iter().try_for_each(|(name, group)| runner::pass(cx, name, group))   // tokens when due, health
}
```

`pool::plan`, `limits::due`, `spend::over`, `life::state`, `idle::quiet_since` and `leave::zero` take values,
the time among them, and touch nothing, so each rule is a table test that waits for nothing.

## Statecharts

Two things have a life here, and they are kept apart. The VM is the provider's: the helper rents it and logs
what it did. The machine is shed's: its states are rows of #309's table, which the helper reads and never sets.

### A lane-host VM: named, pool or image (the helper's log)

```
       start (a named VM: the owner)                       every step passed,
       demand (a pool VM: the helper)                      shed says the machine is up
 absent ─────────────▶ starting ─────────▶ setup ─────────────────▶ running ──┐ a limit is due, or asked:
   ▲                      │                  │                         │  ▲    │ `leave` logged, shed asked
   │                      └────────┬─────────┘                         │  └────┘ to drain (still running)
   │                        failed │                                   │
   │                               ▼                                   │
   │        named ────────────▶ stopped ◀── named: leaving, zero terminals
   │                               │ start                             │
   │                               ▼                                   │
   │                            starting                               │
   └──────── pool, image or `rm`: leaving, zero terminals, destroyed ──┘
```

| from | to | when | what the helper does |
|---|---|---|---|
| `absent` | `starting` | demand, for a pool VM; `start`, for a named one; and the budget allows it | verb `create_from` (a pool VM: the pool's image; a named VM once it has a snapshot), else `create`; logs `start` |
| `stopped` | `starting` | `start` on a named VM, and its budget allows it | verb `start`; logs `start` |
| `starting` | `setup` | the provider says `running`, within the record's `start_secs` | logs `setup` |
| `setup` | `running` | every step passed and shed shows the machine `up` (rows M1, M4) | `ready` (a new VM), `address`, `machine.add` with the key and the identity file, shed's build, a drain the disk carries lifted (D2), the git token; logs `up` |
| `starting`, `setup` | `stopped` or `absent` | the wait ran out, a step failed, or shed gave up on the link (`never-up`, M6) | a named VM: verb `stop`; a pool VM: verb `destroy`; `machine.rm`; logs `stop` or `removed`, why `failed` |
| `running` | `running`, leaving | a limit is due (idle, items, age, budget) or `drain`, `stop`, `rm` asked for it | logs `leave` with why and the end it leads to; `machine.drain` (D1) |
| `running`, leaving | `stopped` | a named VM; shed says the machine drains and lists no terminal | `machine.rm` (M9), verb `stop`; logs `stop` |
| `running`, leaving | `absent` | a pool or image VM, or `rm`; the same | `machine.rm` (M9), verb `destroy`; logs `removed` |
| `running` | `stopped` or `absent` | a pass finds the provider does not say `running` | `machine.rm`; logs `stop` or `removed`, why `outside` |

| state | the log says | notes |
|---|---|---|
| `absent` | no line for it, or the last life line is `removed` | a pool VM's name is not used again |
| `starting` | `start` | spend runs from this line; a pass finishes or fails a start whose process died |
| `setup` | `setup` | the provider says running; shed has not shown the machine `up` yet |
| `running` | `up` | a machine; quiet while the last of `quiet` and `busy` after `up` is `quiet`; leaving once a `leave` line follows |
| `stopped` | `stop` | a named VM only; not a machine; spend ended at this line |

`leave` is the helper's decision, not a state of the machine: it says why the VM ends and how (stop or
destroy), and it stands until the VM ends. Over budget is the one hold of the helper's own: a named VM at its
`budget_usd_month` or `max_hours_day`, or the pools at `[caps] budget_usd_month`. `start` and every create
then refuse `OVER_BUDGET`, what runs is told to leave, and the row's `over` says the sentence. The month or
day turning, or the owner's edit of `config.toml`, lifts it.

Every command is safe to repeat: `start` on a running VM checks its address and exits 0; `drain` on a leaving
one, `stop` on a stopped one and `rm` on an absent one change nothing.

### The machine: rows of #309's table

The chart is [../state-table/README.md](../state-table/README.md)'s; the rows are in
[../state-table/tables.md](../state-table/tables.md). The helper has no branch that sets, times or escalates
one of these states. What it needs, the row that gives it, and how it reads the result:

| the helper needs | rows | it asks shed | it reads |
|---|---|---|---|
| a started VM to be reachable | M1 (`added`, 120 s), M4 (`up`), M6 (`never-up`) | `machine.add` | `machine.ls`: `up` ends setup; `stuck: never-up` fails it |
| a moved address to be dialed | M8 (`relink`) | `machine.add` with `replace` | the same |
| a changed host key to be refused | L6 (`refused`), M10, M19 (`key_changed`) | nothing: the link refuses by itself | `machine.ls`; the owner's `repin` is `machine.add` with the new key (M8) |
| no new terminal on a VM that leaves | D1, D4; M13 (`draining`) | `machine.drain` | `machine.stats`: `draining` |
| a restarted named VM to serve again | D2; M16 | `machine.drain` with `on: false` | `machine.stats` |
| to know a drain is slow | M13's hour, then M18 (`stuck: drain-slow`) | nothing | nothing: drover routes shed's event, as for any machine |
| to know a machine cannot be counted | M5 (`offline`), M14 | nothing | `machine.stats` answers `NO_LINK` at once: nothing is decided |
| the machine gone with its VM | M9 (`removed`, by `shed-cloud`) | `machine.rm` | - |
| zero terminals | T27, T34: a name is `free` only after `rm` or 24 h | nothing | `machine.stats`: no terminal in any state (a warm spare is none) |
| quiet | the terminal states | nothing | `machine.stats`: none `starting` or `running` |

The rows this issue owns, built in its slices 1 and 2 exactly as #309's tables have them (those slices add
them to the flow files if #309's slice 1 left them for here), with their readers and actions in `drive` from
their first day:

| slice | facts | states | rows | actions and edge |
|---|---|---|---|---|
| 1, the pinned key | link: `refused` read from a host-key refusal; machine: `change`, `key_mismatch` | `key_changed` | M8, M10, M19, and the `change` and `key_mismatch` cells of M1 and M3 | `relink`, `hold`; the edge passes a differing `machine.add` only with `replace` |
| 2, draining | the drain flow's `drain`, `lift`; machine: `draining`, `serving` | the drain flow's two; `draining` | D1 to D5; M13, M16, M17, M18; `draining` in M5, M7 to M10; the `draining` and `serving` cells of M3, M11, M12, M15, M19 | `end-spares`; the edge refuses `run` with `DRAINING` |

The rest of the machine's life (`added`, `up`, `offline`, `removed`: M1, M2, M4 to M7, M9, M14, M20) is #309's
own slice 5. This issue adds no row beyond #309's 106; a slice that finds it needs one changes the flow file,
under the gap check, not the helper.

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

A runner VM is no shed machine, so none of #309's rows is about it: its `draining` is the runner service's
own. `low` is reported, not repaired: the pass revives nothing by itself, as today. The pass also reads each
VM's `state`; one the provider no longer runs (its TTL passed) is logged `removed`, why `outside`, and stays in
the group's count until `runner rm`. `runner add` refuses `RUNNERS_CAP` past `max_vms` or `usd_day`; `runner rm`
of the last VM with online runners refuses `RUNNERS_LAST` without `--last`. A runner VM's spend is its rate
times the time from its `runner-add` line to its `removed` or `runner-rm` line.

## The CLI

Exit codes: 0 done, 1 an error (a verb failed: its stderr, with secrets cut out), 2 usage, 3 refused
(`REFUSED <CODE>: why` on stderr). Every command that prints takes `--json`.

| command | does | prints |
|---|---|---|
| `ls` | reads the log, the config and the demand files; asks no provider, no GitHub and no shed | one row per VM, pool and runner group, and the caps ([shapes.md](shapes.md)) |
| `demand --by <who>` | takes the lanes waiting, by tags, from stdin; stores it; sizes the pools; starts each create as its own process | what it created and what a cap held back (`Plan`); refuses `BAD_DEMAND` |
| `drain <vm> [--now]` | decides the VM leaves and asks shed to drain its machine; the pass ends the VM at zero terminals. `--now`, the owner's, ends it at once | `<vm> leaving (<n> terminals)`, or how it ended |
| `start <vm>` | a named VM: create or resume, wait for running, set up, wait for shed to show the machine up; a failure stops it; on a running VM, only the address check | `<vm> running <user@host:port> (machine <m> up)`; refuses `OVER_BUDGET`, `KEY_CHANGED`, `NO_VM`, `NOT_NAMED`, `SHED_OLD` |
| `stop <vm> [--now]` | a named VM: leaves, then at zero terminals verb `stop` and `machine.rm` | `<vm> leaving (<n> terminals)` or `<vm> stopped`; refuses `NOT_NAMED` |
| `rm <vm> --yes [--now]` | leaves, then at zero terminals verb `destroy` and `machine.rm` | `<vm> leaving (<n> terminals)` or `<vm> removed` |
| `repin <vm>` | for a machine shed holds as `key_changed`: `machine.add` with the key `address` prints now | `<vm> repinned` and both key fingerprints |
| `snapshot <vm>` | a named VM: needs `running` and the machine up; verb `snapshot`; waits for the machine again | `<vm> snapshot saved` |
| `image build <pool>` / `image ls` | builds the pool's golden image now / lists each pool's images with key, age and state | each step as it passes; refuses `IMAGE_DIRTY` |
| `tick` | the pass | one line per change (`dev-16: removed (idle 20 min)`); nothing when nothing changed |
| `log [--since T] [<vm>]` | the log's lines | `Line`s |
| `runner add <group> [n]` | create and set up n VMs | `<vm>: created (<size>), <slots> runners starting`; refuses `RUNNERS_CAP` |
| `runner rm <group> <vm> [--last]` | drain, deregister, destroy | `<vm>: drained, <n> runner(s) deregistered, deleted`; refuses `RUNNERS_LAST` |
| `runner revive <group> <vm>` | a fresh token and a service restart | `<vm>: fresh registration token, runner service restarted` |
| `runner tokens <group>` | a fresh token to every VM now | `tokens: a fresh one on <n>/<m> VMs` |
| `secret set <NAME>` / `secret ls` | saves stdin as a 0600 file / lists names and dates | never a value |
| `doctor` | rows: each record's key and identity file, its CLI's subcommands against `--help`, an override missing a key the built-in has, a pool with no image, `[git]` with no key, the job, shed's protocol here and on each running VM | `✓`, `!`, `✗` rows; exit 1 on a `✗` |
| `install` / `uninstall` | writes, loads or removes the job `dev.shed.cloud` | what it wrote |
| `import <config.toml> <log.jsonl>` | takes both files once, when its own log is empty | what it took |

shed's own CLI gains `shed machine add --host-key --identity --replace` (slice 1) and
`shed machine drain [--on M] [--off] [--why TEXT]` (slice 2).

## Module boundaries

```
  shed-cloud (binary) ──▶ shed-client ──▶ shed-proto     machine.add, machine.rm, machine.ls, machine.stats,
        │        │                                       machine.drain, ls, run, rm, exec, hello
        │        └──────▶ shed-core                      runner (every process), clock
        ▼
  sh -c <a record's line>, gh, a pool's `key` command    all through shed_core::runner

  shed, shed-server and every other shed crate: no use of shed-cloud, no mention of it
  drover: runs `shed-cloud ls --json` and `shed-cloud demand`; links nothing of it; calls shed for the rest
```

| file in `crates/shed-cloud/src/` | holds | may call |
|---|---|---|
| `main.rs` | the real `Cx`: the clock read once, the paths, shed's socket; then `lib.rs`'s `run` | `lib` |
| `lib.rs`, `cmd/<verb>.rs` | `run(cx, args)`: arguments in, text or JSON out; one file per command | every module below |
| `out.rs` | the one stdout and stderr writer | nothing |
| `paths.rs` | the helper's config and state folders and shed's socket; the one reader of the environment | nothing |
| `config.rs` | `Config`, `Caps`, `Pool`, `Named`, `Group`, `Git`, the checked names | nothing |
| `record.rs` | a provider record and a git record: load, check, fill a line | `exec` |
| `exec.rs` | run a filled line: the key and a token on stdin, stderr with both cut out | core (`runner`) |
| `secret.rs` | `Secret`; the environment, else the 0600 file; `secret set` | `paths` |
| `log.rs` | `Line`, `Event`; append, read, the per-VM lock | `paths` |
| `shed.rs` | the one user of shed-client. Asks: `add`, `drain`, `lift`, `rm`, `exec`. Facts: `Counts` (from `machine.stats` and `ls`), `up` and `stuck` (from `machine.ls`) | client |
| `life.rs` | `state`, `leaving` (pure); start, set up, `end` (which takes a `Drained`), snapshot; a start that died | `record`, `log`, `shed`, `address`, `setup`, `spend`, `git` |
| `leave.rs` | `Drained` and its three makers (`zero` is pure); `decide`: the `leave` line, then `shed.drain` | `shed`, `log` |
| `spend.rs` | spend, hours today, `over`; pure | nothing |
| `address.rs` | parse `address`; `keep`: another address with the same key is `shed.add` again | `record`, `shed`, `log` |
| `idle.rs` | `Counts` into `quiet`, `busy` and `item` lines; `quiet_since` (pure) | `log` |
| `limits.rs` | `due`: idle, items, age, budget; pure | `spend` |
| `demand.rs` | `Demand`: parse, store, the fresh ones | `paths` |
| `pool.rs` | `plan` (pure); `size`: the creates, each a detached start | `demand`, `life`, `log`, `spend` |
| `image.rs` | the key; the build; `never`; the newest two kept | `record`, `life`, `git`, `shed`, core (`runner`, for the sums and the `key` command) |
| `git.rs` | mint on the main host, install and remove through shed's `exec`, `refresh` | `record`, `secret`, `shed`, `log` |
| `setup.rs` | a VM's shed step: the release asset through the record's `exec` when the build differs | `record`, core (`runner`, for the sums) |
| `runner.rs` | a group: GitHub's list, a registration token, register, tokens, health, drain, deregister | `record`, `log`, core (`runner`, for `gh`) |
| `tick.rs` | the pass | `life`, `leave`, `address`, `idle`, `limits`, `pool`, `image`, `git`, `runner`, `shed` |
| `service.rs` | the job, through farrier-service | farrier-service |

Rules, each with its enforcer:

- No shed crate uses `shed-cloud`, and it uses core, proto and client only (the `cloud` layer; the
  `shed-cloud` ban; the `cloud-free` gate on `cargo tree`).
- It names no provider: every provider fact is a record (the `boat` ban).
- It holds no machine state. Only `shed.rs` talks to shed, and what it hands the other modules is `Counts`,
  `up` and `stuck`: no module can compare a machine's state, and none sets or times one (the `shed_client` and
  `flow::` bans, with `shed.rs` their one exception; `life::State` has five values, all the provider's).
- The time is a value. `main.rs` reads `shed_core::clock::unix_now` once into `Cx.now`, and every rule and
  every log line takes that number. The helper compares it only with its own log's times, never with a time
  shed gave. A wait on an event (the provider says running, shed shows the machine up) is `clock::poll`,
  bounded by the record's seconds (a `unix_now` ban in the crate outside `main.rs`; the existing clock bans,
  which cover `crates/*/src/**` and, after #310, `crates/*/tests/**`).
- It starts every process through `shed_core::runner` (the existing ban).
- It reads the environment in `paths.rs` only (the `env::var` ban's one new exception). A test holds
  `paths::socket()` equal to `shed_server::Paths::user()`'s under the same variables.
- A VM ends only through `life::end`, which takes a `Drained` (the type: its field is private to `leave.rs`).
- `spend.rs` and `limits.rs` do no I/O, and `pool::plan`, `life::state`, `leave::zero` and `idle::quiet_since`
  take values and return one: table tests, with drover's cases from `crates/cli/tests/cloud.rs` and `runners.rs`.

In shed's server, on #309's layout: slice 1 touches `machines.rs` (the entry's two new fields, one function
that builds the ssh options, used by the dial and by the proxy forward in `proxy.rs`), `flows/link.rs` (a
host-key refusal read as `refused`), `flows/machine.rs` (`change`, `key_mismatch`, `relink`, `hold`),
`methods/machine_add.rs`, `shed-proto`'s `machine.rs` and `shed`'s `cmd/machine.rs`. Slice 2 adds
`methods/machine_drain.rs` and `flows/drain.rs` (the drain flow's facts, `end-spares`, `drain.json`), the
edge's check under the registry's `run` lock, one field in `stats.rs`, the stats reader in `flows/machine.rs`,
and the same two proto and CLI files. Nothing in either is about VMs.

## Secrets

| rule | held by |
|---|---|
| A secret is a `Secret`: no `Debug`, `Display`, `Serialize` or `Clone`. | the type; a compile-fail doc test |
| A provider's key and a git record's key are the variable the record names (`key_env`), else the 0600 file `secrets/<NAME>` in the helper's config folder. A file wider than 0600 is not read. | `SECRET_FILE_OPEN`; test; a doctor row with the `chmod` |
| `secret set` reads the value from stdin; `secret ls` gives names and dates. There is no get. | the CLI; test: every command's output after a set, searched for the value |
| A key reaches a line as a line on its shell's stdin, exported there under `key_env`. It is in no argument and in no environment the helper sets. | test: the runner's recorded arguments and environment |
| A registration token comes from `gh api` on the main host, under `gh`'s own login, and lives in one command's memory. It reaches the record's `exec` as a line on stdin, exported there as `$RT`, and no argument on the main host holds it. | test: the arguments and bytes of each process; the log and the output searched for the token |
| A git token comes from the git record's `mint` on the main host (a GitHub App's one-hour token) and lives in one command's memory. It reaches the VM as the stdin of shed's `exec`, over the pinned link, and is in no argument, no `exec` environment and no file on the main host. | test: the `exec` params the fake server saw, and the fake link's `sent.bytes`; the log and the output searched for the token |
| No provider key, no App key and no agent login ever goes to a VM. An image is not saved while a `never` path exists. | the helper sends a VM only tokens; `IMAGE_DIRTY`; test with each `never` path present |
| A line's stderr is returned with the key and the token cut out. | test with a line that prints both |
| The log holds no secret: a host key is public, and no line has a field for a key or a token. | the `Event` type |
| Every value filled into a line matches its checked form first; `$C` and stdin are never filled in. | the checked name types; test with quotes, spaces and `$(…)` in each field |
| A verb that needs the VM's id and gets nothing does not run (boat reads an empty id as the VM it runs in). | `NO_VM`; test, moved from drover |
| `Secret::expose` is called in `exec.rs` only. | the `.expose()` ban's one new exception |
| No credential of ours goes to a runner VM, and no shed. | the runner path calls `exec` and `gh` only; test: the bytes of a `runner add` |

The App's key is the one new secret (`GITHUB_APP_KEY`, set with `secret set`). Pushes and PRs from a VM show
the App as their author. Not covered: the `boat` CLI's own login, which the helper never reads; and an
unsandboxed lane on the main host, which runs as the owner's user and can read a 0600 file (#259's security
model says the same of shed's own). A lane on a VM can read that VM's git token: it is what the lane pushes
with, it reaches only `repos`, and it dies within the hour.

## Tests

This issue builds no fake and no clock of its own.

| what | how | whose |
|---|---|---|
| the link to a VM | `scripts/fake-link.py` stands where `ssh` stands, as in shed's machine tests. The test's provider record is shell lines over the fake link's hosts folder: `create` starts a shed server in a new folder and puts its `shed` under `<hosts>/<target>/`; `address` prints the target and a key; `destroy` stops the server and writes `<target> gone` to the faults file. A resumed VM is `moved` and `rekey` | #308 |
| a fault while a VM is rented | the same faults file: `hang`, `cut` and `gone` under a leaving VM (nothing ends without a count), `moved` (one `machine.add`, the terminals kept), `rekey` (shed holds the machine; the helper changes nothing until `repin`) | #308 |
| the machine's states | asserted on shed's `machine.ls` and its `flow` events; the helper has none to assert | #309 |
| idle, budget, age, the month's turn, a token's refresh | the test calls `shed_cloud::run` with a `Cx` whose `now` it sets: a pass at `t`, a pass at `t + 21 min`, and `leave`, why `idle`, is in the log. The time is never the wall's, and nothing sleeps | #310 |
| a wait on an event (a machine up, a VM gone) | `clock::poll(clock::PATIENCE, …)`; a bound is never a pass or fail line | #310 |
| a verb's wait running out | the test record's `start_secs = 0` | the record |
| the built binary | one test per refusal code and exit code, and the secrets table above | this crate |

```rust
let world = World::new("idle");                       // a temp folder; the fake link first on PATH; a main shed
let mut at = Clock(1_800_000_000);                    // a number: the helper's only time
world.demand(&at, "ci", r#"{"waiting":[{"tags":["linux"],"lanes":1}]}"#);
let vm = world.wait_running("dev-1");                 // clock::poll on the helper's log
world.pass(&at);                                      // no terminal: logs `quiet`
at.add(mins(21));
world.pass(&at);                                      // idle is due: `leave`, and shed is asked to drain
assert!(world.stats("dev-1").draining.is_some());     // the drain flow's row D1, read from shed
world.pass(&at);                                      // shed lists no terminal: `machine.rm`, verb `destroy`
assert_eq!(world.log_kinds(&vm), ["start", "setup", "up", "quiet", "leave", "removed"]);
```

The pool VM joins #308's nightly soak in slice 5. `scripts/soak.py` takes one more kind of target,
`LABEL=pool:<name>`: it publishes demand for one lane as `--by soak`, waits for the VM, copies its machine
entry (target, key, identity) from the main host's shed into the soak's own shed, measures it like any
machine, then runs `shed-cloud drain`, removes its terminals, waits for the VM and its machine to be gone and
publishes no demand. `ci/soak.toml` gains two targets, `vm_ready_s` and `vm_gone_s`, and `soak.yml` passes the
pool named in the repository variable `SOAK_POOL`. The cost is one pool VM for a few minutes a night, inside
`[caps]`. A lane drover placed on that VM meanwhile is not a miss: `vm_gone_s` is then reported as skipped.

## Migration

| step | repo | what | needs |
|---|---|---|---|
| 0 | shed | #308 (merged), #310, and #309's slices 1 and 2: the tables, the engine, the runner | - |
| 1 | shed | slice 1: a machine's port, pinned key and identity; `machine.add` replaces; `key_changed` | 0 |
| 2 | shed | slice 2: draining | 0 |
| 3 | shed | slice 3: the helper, by hand: records, start, drain, stop, rm, repin, snapshot, spend, `ls`, `log`, `secret`, `doctor`, the checks | 1, 2, #309's slices 3 and 5 (the terminal counts by state, `machine.ls`), the cap issue |
| 4 | shed | slice 4: the pass, idle, the job, the release asset and self-update, `import` | 3 |
| 5 | shed | slice 5: pools, `demand`, items, the git token, the pool VM in the soak | 4 |
| 6 | shed | slice 6: the golden image | 5 |
| 7 | shed | slice 7: runner groups | 3 |
| 8 | drover | runners move (drover#1644): `drover runners` calls the helper or is deleted; `runners.rs` deleted | 7 |
| 9 | drover | lane hosts move (drover#1644): pool rows as hosts, `demand`, a lane placed where its item is warm, `draining`, save then remove, the image script and its key command, the export; `cloud.rs` deleted | 6, drover#1575 |

Slices 1 and 2 can run side by side, and so can 4 and 7. drover's multi-host harness (drover#1648) builds on
#308 and needs nothing from here. Until step 9 the helper's config is empty and it does nothing, so two
programs never end the same VM. Step 9 carries the live state, in drover's own rollout:

1. The owner writes `[caps]`, a `[pool.<name>]` and `[git]`, sets the keys, and builds the first image.
2. drover writes the helper's two files from its own: `config.toml` from `[hosts.<n>]` and `[ci_runners]`, and
   `log.jsonl` from this month's `cloud-*` and every `runners-add` and `runners-remove` event
   ([shapes.md](shapes.md) has the key and event tables). drover owns that export: it knows its log.
3. `shed-cloud import` checks both with its own types and takes them. It refuses when its log is not empty.
4. The next pass finds each of today's VMs `running` in the log and at the provider, as a named VM, reads its
   address, and adds the machine with the key drover had pinned. No VM restarts, lanes on them run on, and this
   month's spend is the same number. They stop when idle and stay stopped; from then the pool takes new lanes,
   and the owner removes them with `rm` when they are no longer wanted.
5. drover's release from then on has no cloud pass. The owner removes the cloud keys from `fleet.toml`.

An owner's recipe override is moved by hand to the helper's `providers/` folder; `doctor` flags one that still
has drover's `runner_*` keys. The owner's fleet has no override and no `[ci_runners]` today (read 2026-11-23),
so step 8 carries no live state there.

## The issues to file

| # | title | scope | src lines |
|---|---|---|---|
| 1 | machines: a port, a pinned host key and an identity file | `ssh://user@host:port` targets tested; `Machine` and `MachineParams` gain `host_key` and `identity`, `MachineParams` gains `replace`; the config line's table form; one ssh-options function for the dial and the proxy forward (`HostKeyAlias`, a known-hosts file per machine in the control folder, `StrictHostKeyChecking=yes`, `IdentitiesOnly=yes`); the rows, readers and actions of the table above; the fake link's `rekey` takes the new key, so a re-pin to it connects; `shed machine add --host-key --identity --replace`; `PROTOCOL` + 1; `docs/commands/machine.md` | +90 |
| 2 | machines: draining | `machine.drain`; the drain flow and `drain.json`; the edge refuses `run` with `DRAINING`; `end-spares`; `MachineStats.draining` and the machine flow's reader of it; a warm spare counted as `spare` in the stats, not as a terminal; `shed machine drain`; `PROTOCOL` + 1; `docs/commands/machine.md` | +70 |
| 3 | shed-cloud 1: the helper, by hand | the crate, its layer, bans and gate; `providers/boat.toml` from drover's recipe; `record`, `exec`, `secret`, `log`, `shed`, `life`, `leave`, `spend`, `address`, `setup`; `ls`, `start`, `drain`, `stop`, `rm`, `repin`, `snapshot`, `log`, `secret`, `doctor`; the test world over the fake link; drover's cloud tests as table and binary tests; `docs/cloud/vms.md`, `providers.md`, `budget.md`, `machines.md` | +700 |
| 4 | shed-cloud 2: the pass, the job and the release | `tick`, `idle`, `limits`, a start that died; `install` and `uninstall`; `release.yml` builds, signs and publishes `shed-cloud-<target>`; `scripts/install.sh` takes it; `shed self-update` keeps the release's binaries installed beside `shed`; `import`; `docs/cloud/pass.md`; the README's Scope names the helper | +330 |
| 5 | shed-cloud 3: pools and demand | `[caps]`, `[pool]`; `demand`, `pool::plan` and `size`; items; a pool VM's limits; pool rows in `ls`; `git.rs` and `git/github-app.toml`; `soak.py`'s pool target and its two targets in `ci/soak.toml`; `docs/cloud/pool.md`, `tokens.md` | +330 |
| 6 | shed-cloud 4: the golden image | `image.rs`; `image build` and `image ls`; the key; `never`; the newest two kept; a bad image set aside; `docs/cloud/image.md` | +200 |
| 7 | shed-cloud 5: runner groups | `providers/boat-api.toml` with its `[runner]` lines; `runner.rs`; `runner add`, `rm`, `revive`, `tokens`; group rows in `ls`; drover's runner tests; `docs/cloud/runners.md` | +260 |
| cap | cap: raise for shed-cloud | one raise, sized by `drover cap berth` on the day; filed when slice 3 starts | 0 |

drover's side (delete `cloud.rs` and `runners.rs`, the export, pool rows as hosts, `demand`, placement by item,
the save and remove on a draining machine, the image script, its docs) is drover#1644's to slice;
[port-map.md](port-map.md) lists what stays there.
