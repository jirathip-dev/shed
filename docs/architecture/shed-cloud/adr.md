# ADR: shed-cloud, an optional helper binary that rents VMs; shed's server stays VM-free (#307)

## Summary

Decision: `shed-cloud` is a new crate and binary in shed's repo that nothing else in shed depends on. It rents VMs from one data file per provider (Boat today) and runs by itself every 5 minutes on the main host. Lane hosts come from a warm pool: drover publishes how many lanes wait for which tags, and the helper creates disposable VMs from a golden snapshot within its own caps (VMs at once, VMs per tag, money per month) and adds each as a shed machine with its address and pinned host key. A pool VM takes 2 issues by default, counted by the `item` label on its terminals, so one issue's impl, review and fix rounds stay on one warm VM; it is deleted when their terminals are gone, when it is idle, or at its age limit. Before it ends a VM the helper asks shed to drain the machine, and the VM goes only when shed lists no terminal on it. A VM you name yourself is started by you only, with its own monthly budget and idle stop. CI runner VMs use the same helper and are never shed machines. drover starts and stops nothing.
The helper keeps no machine state of its own: draining, key changed, added, up, offline and removed are rows of shed's transition table (#309); the helper reads them as facts and asks shed for the actions. Its tests run over #308's fake link, and its idle, budget and age rules take the time as a value (#310), so no test waits on a real clock.
Slices: (1) machines: a port, a pinned host key, an identity file; (2) draining; (3) the helper, by hand; (4) the 5-minute pass, its job and its release; (5) pools, demand, the per-VM git token, the pool VM in the nightly soak; (6) the golden image; (7) runner groups. Slices 1 and 2 need #309's slices 1 and 2; slices 3 to 7 also need #309's terminal and machine take-overs. Then drover's own issues (drover#1644, drover#1648). About 2,000 lines added in shed, about 850 leave drover.
Open questions: none. The owner's answers of 2026-11-23 are decisions here: a GitHub App mints each VM's git token, a named VM is started by the owner only, cooee's desks stay in cooee, and the cap raise is its own issue, filed when slice 3 starts.

**Status:** approved by the owner, 2026-11-23.
Revised by the architect lane the same day, after the owner asked for a revision: issues
counted by the `item` label, the fake link and the soak moved to #308, the machine's states left to #309's
table, time as a value under #310, and the four open questions answered. Read on shed staging `c1fba0b` and
drover staging `7c0a28c1c`. It follows the owner's decisions of the same day on drover#1644: cloud VMs leave
drover for an optional helper in shed's repo, with a demand-driven warm pool, drain first and a golden
snapshot. Where each part is written:

| file | holds |
|---|---|
| [README.md](README.md) | how the callers read, the statecharts and the rows of #309's table behind them, the CLI, the module boundaries, the secrets, the tests, the migration, the issues to file |
| [pool.md](pool.md) | demand, sizing, a pool VM's life, `draining` in shed, persistent hosts, the golden image, sign-in and the git token |
| [shapes.md](shapes.md) | config, provider record, log and wire shapes; the import from drover |
| [port-map.md](port-map.md) | every function of drover's `cloud.rs` and `runners.rs` |

## The question

drover rents VMs itself, in two files: `cloud.rs` (669 lines: recipes and keys, create and resume, stop, the VM's
changing address and host key, bootstrap and snapshot, spend and the budget hold, the idle-stop pass, doctor rows)
and `runners.rs` (359 lines: runner VMs, registration tokens, health). drover#1644 counts 15 dependencies on
`cloud` inside drover (spawn, hosts, check, update, doctor, the board). A resumed Boat VM comes back with a new address and a new host
key; drover#1644 records that stop and resume broke lanes on 2026-11-23, and the owner's fleet keeps its hosts up
600 minutes to avoid a resume.

The accepted hosts ADR (#259, one day older) moved the VM mechanism into shed's server as `vm.*` methods and
left budgets, idle stop and runner registration in drover. The owner has since decided otherwise: one optional
helper owns all of it, shed's server knows machines only, and lane hosts are disposable VMs the helper sizes
from drover's demand. So: where does the helper live and what may it use, what does it keep on disk, how is a
provider described, how does a VM stay a shed machine, how does drover ask for capacity without starting a VM,
how does a VM end without losing a lane's work, what is in the snapshot a VM starts from, and who holds the
states of the machine a VM is?

## Options

**Where the VM code lives**

| | option | verdict |
|---|---|---|
| A | In shed's server, as `vm.*` methods (#259) | reversed by the owner: the server would rent VMs and hold a provider's key |
| B | A separate repo | the owner chose shed's repo: one release, one protocol window, shed's gates |
| C | **A crate and binary in shed's workspace that no other crate may use** | chosen |

**How a provider is described**

| | option | verdict |
|---|---|---|
| D | A Rust trait with a Boat implementation | a new provider is code and a release; Boat's CLI and HTTP API would be re-typed in Rust, with an HTTP client the workspace does not have |
| E | **An exec contract: one TOML record per provider, each verb a `sh -c` template with a fixed output** | chosen: it is drover's recipe, which already carries the fixes of about 31 issues; a new provider is one data file; #259 chose the same |
| F | One helper program per provider (`shed-cloud-<name>`) | a second binary to ship and update for what is ten shell lines |

**How the helper talks to shed**

| | option | verdict |
|---|---|---|
| G | Run the `shed` CLI and parse its output | a `shed` on PATH of another build, text to parse, a fake binary in every test |
| H | **`shed-client`, as cooee and drover do** | chosen: typed calls, the protocol handshake, the generated flow states |

**How a VM's address and host key reach shed**

| | option | verdict |
|---|---|---|
| I | The helper writes a block in `~/.ssh/config.d/` and shed keeps an alias (drover today) | needs an `Include` the owner adds by hand; the address lives in a second place; #259 already retired it |
| J | shed dials whatever the helper tells it per call | every caller of the machine would need the helper |
| K | **shed's machine entry holds the target with its port, the pinned key and the identity file; the helper replaces the entry when they change** | chosen: three ssh facts any machine can have, about 90 lines in the server, nothing about VMs |

**Who holds the states of the machine a VM is**

| | option | verdict |
|---|---|---|
| L | The helper: its own `draining` state, its own holds (`key_changed`, `unreachable`, `drain_stuck`) and their timers (this ADR's second draft) | replaced by the owner: a second place that decides up, down and draining is root cause 2 of #309 again |
| M | **shed's table (#309). The helper reads a machine's state and its terminal counts as facts and asks shed for `machine.add`, `machine.drain` and `machine.rm`** | chosen: one set of rows, timers and escalations for every machine, rented or not; the helper keeps the provider's side only (starting, setup, running, stopped) and why a VM is to leave |

**Where the helper's own state lives**

| | option | verdict |
|---|---|---|
| N | A state file with counters (seconds this month, last busy) | a crash between a provider call and the write leaves it wrong; no record of why a VM stopped |
| O | **An append-only log; state, spend and idle are functions of it and of the time** | chosen: as drover does today, so this month's spend imports exactly, and each rule is tested as a table |

**How idle is known**

| | option | verdict |
|---|---|---|
| P | drover tells the helper which hosts have lanes | the helper would depend on drover running |
| Q | **`machine.stats`: no terminal on the machine is `starting` or `running`** | chosen: a lane is a shed terminal, and so is anything else worth keeping a VM up for. A `done` or `asleep` terminal does not keep a VM up (it is drover#1597's "done counts as live" otherwise), but it still counts toward zero |

**What a VM waits for before it ends**

| | option | verdict |
|---|---|---|
| R | A `before_stop` command per VM that drover sets to its save (this ADR's first draft) | replaced by the owner's drain first: a hook runs once, at a moment the helper picks, while a lane may still be writing |
| S | **Drain: no new terminal, then the end at zero terminals. A finished terminal counts until its owner removes it** | chosen: drover saves a lane's work when the lane ends and then removes its terminal; the removal is its "done here", so no call between the two programs is needed |

**Where `draining` lives**

| | option | verdict |
|---|---|---|
| T | A flag in the main host's machine list, checked where the main host forwards `run` | `shed --on`, a second main host and a shell on the VM are not checked; a `run` already on its way can land after the helper read zero |
| U | **The drain flow of the shed server on the machine itself (#309's rows D1 to D5)** | chosen: one place refuses, under the lock that creates terminals, so zero stays zero; any machine can be drained by anyone, a persistent host before a reboot included |

**How drover gets capacity**

| | option | verdict |
|---|---|---|
| V | drover runs `shed-cloud start` when placement wants a VM (the first draft) | reversed by the owner: drover never starts or stops a VM |
| W | drover writes a demand file in the helper's state folder | a second writer in the helper's folder, and nothing happens until the next pass, up to 5 minutes later |
| X | **`shed-cloud demand`: the lanes waiting, by tags, on stdin; the helper sizes at once and answers what it started and what a cap held back** | chosen: one command, checked input, no wait for a pass, and drover's board can say why a lane still waits |

**What `max_lanes` counts**

| | option | verdict |
|---|---|---|
| Y | The terminals shed lists on the machine, each name once (the second draft) | replaced by the owner: an issue's impl, review and fix rounds are three terminals, so one issue used a VM up and its next round started cold |
| Z | drover reports the issues per machine | the helper would need drover to end a VM |
| AA | **The distinct values of the `item` label on the machine's terminals** | chosen: drover's spawn already labels a lane's terminal with its repo (`group`) and its issue (`item`); the helper counts the pairs and never reads what they mean. A terminal with no `item` counts as none |

**What rebuilds the golden image**

| | option | verdict |
|---|---|---|
| AB | A timer | rebuilds when nothing changed, and is stale in between |
| AC | **A key: the hash of the build script, of a command's output (lockfiles, toolchain files), of shed's build and of the pool's size** | chosen: the same inputs give the same snapshot name, so "is it current" is a comparison |

**How a rule about time is tested**

| | option | verdict |
|---|---|---|
| AD | The built binary under the real clock, with log lines stamped in the past | no sleep, but the result still depends on the wall: a run at a month's first minute counts another month's spend |
| AE | **The time is a value: the binary's `main` reads the clock once, and a test calls the same entry with the number it chooses (#310)** | chosen: idle, budget, age and a month's turn are passes at two numbers; as #309's engine does |

**Is a runner VM a shed machine?**

| | option | verdict |
|---|---|---|
| AF | Yes, like a lane host | shed would be installed on a VM that runs pull-request code, with a held link from the main host that can carry the token-proxy forward (#265) |
| AG | **No: it is reached only through the provider's `exec`, and nothing of ours is on it** | chosen: as today, and as drover#1644 words it ("a shed machine or a GitHub runner") |

## The decision

1. **Where it lives.** Crate `crates/shed-cloud`, binary `shed-cloud`. It uses `shed-core` (to start
   processes and read the clock), `shed-proto` and `shed-client`, and the workspace's `clap`, `serde`,
   `serde_json`, `toml`, `anyhow` and `farrier-service`. No new dependency. No crate uses it. Three checks hold
   that: an `arch.toml` layer no other layer lists, a ban on its name in every other crate's source, and a gate
   that fails when `cargo tree -p shed` names it or `cargo tree -p shed-cloud` names the server.
2. **Providers.** A provider is a record, `crates/shed-cloud/providers/<name>.toml`, built in; the owner's file
   of the same name in the helper's config folder replaces it. Verbs: `create`, `start`, `stop`, `state`,
   `destroy`, and where it has them `id`, `address`, `ready`, `snapshot`, `snapshot_rm`, `create_from`, `exec`.
   Two records ship: `boat` (the CLI, for lane hosts) and `boat-api` (the HTTP API, for runner VMs), both from
   drover's recipe. The contract is in [shapes.md](shapes.md). The code names no provider.
3. **Config and state.** The owner's `config.toml` has the caps, each pool (`[pool.<name>]`), each named VM
   (`[vm.<name>]`), each runner group (`[runners.<name>]`) and the git token's record (`[git]`). The helper
   writes one log, `log.jsonl`, and one small file per demand publisher. A VM's state at the provider, its
   spend, the issues it has had, how long it has been quiet and whether it is to leave are computed from the
   log, the config and the time, each time.
4. **Budget.** Spend is the time a VM is up (from its `start` line to its `stop` or `removed` line), within the
   UTC month, times the record's rate for its size. A named VM has its own `budget_usd_month` and does not load
   without one. Pool VMs and image builds share `[caps] budget_usd_month`. At a budget nothing new starts
   (`OVER_BUDGET`) and what runs is told to leave.
5. **The machine's states are shed's.** A VM that runs is a shed machine, and every state of that machine
   (`added`, `up`, `offline`, `draining`, `key_changed`, `removed`), with its timer and its escalation, is a
   row of #309's table. The helper has no such state, hold or timer. It reads `machine.ls` and `machine.stats`
   through one file, and asks shed for three actions: `machine.add`, `machine.drain`, `machine.rm`.
   [README.md](README.md) lists each need with its rows, and the rows this issue's slices 1 and 2 build.
6. **The machine link.** After a start the helper reads the record's `address` and calls `machine.add` with the
   target `ssh://user@host:port`, the key it prints and the identity file; shed pins the key. Each pass reads
   `address` again: a moved address with the same key is `machine.add` again (row M8). A key that changes
   under a running VM is refused by shed's own link (rows L6, M10): the machine is `key_changed` until the
   owner runs `repin`. The end of a VM, by anyone, removes the machine. So shed never dials an address a VM
   gave back.
7. **Idle.** Each pass reads the machine's terminal counts. None `starting` or `running` logs `quiet` once; one
   logs `busy`. A VM quiet for its idle limit (20 minutes by default) is told to leave. A machine that does
   not answer is not quiet.
8. **Drain first.** `machine.drain` puts the shed server on a machine in its `draining` state: the edge refuses
   `run` with `DRAINING`, the warm spares end, and `machine.stats` says so. Everything else works, so lanes
   finish and work is saved. The helper decides a VM is to leave (`leave`, with why), asks shed to drain it,
   and ends it only on a stats answer that says `draining` and lists no terminal in any state. drover reads
   the machine's `draining` from shed, places nothing there, and removes a lane's terminal after it saved the
   lane's work. A drain still unfinished after an hour is shed's `stuck: drain-slow` event (M13, M18), which
   drover routes. [pool.md](pool.md).
9. **The pool.** `shed-cloud demand --by <who>` takes the lanes waiting, by needed tags. The helper gives each
   entry to the cheapest pool whose tags cover it, subtracts the room that pool already has or is starting,
   and creates VMs from the pool's image within the pool's `max_vms`, `[caps] max_per_tag`, `[caps] max_vms`
   and the budget. `max_lanes` (2 by default) counts issues: the distinct values of the `item` label (with `group`) on
   the VM's terminals. A VM at that count takes no new issue, and every later lane of its issues still fits. It
   leaves when no terminal is left on it, after `idle_minutes` quiet, at `max_life_hours`, or at the budget,
   and is deleted at zero terminals. Demand never removes a VM. A persistent host is a plain shed machine the
   helper does not know. [pool.md](pool.md).
10. **The golden image.** One saved snapshot per pool, named by its key. The pass rebuilds it when the key
    changes: a fresh VM, the pool's build script in a shed terminal, a check that none of the record's `never`
    paths exist, the provider's `snapshot`, the VM deleted. It holds repos, toolchains, shed and a warm
    dependency build. It never holds a login, a token or a key. [pool.md](pool.md).
11. **Sign-in and git.** No agent logs in on a VM: a `run` there gets its credentials through shed's token proxy
    (#265), which the held link already forwards. Git and `gh` on a VM use a one-hour token that a GitHub App
    mints for that VM on the main host; the helper sends it over the shed link on stdin and replaces it every
    45 minutes. The App's key is one new secret on the main host, and pushes from a VM show the App as author.
    How a token is minted is a record (`crates/shed-cloud/git/github-app.toml`).
12. **Named VMs.** Only the owner starts one. Demand never wakes a stopped one; the pool answers demand.
13. **Runner VMs.** A group's VM is created, then set up: a registration token from `gh` on the main host, sent
    on stdin through the record's `exec`, and the runner service restarted. The pass sends a fresh token every
    30 minutes and reads how many of the VM's runners GitHub lists online. Revive, add and remove are commands.
    `max_vms` and `usd_day` are hard: only an edit of `config.toml` raises them.
14. **Secrets.** A provider's key and the App's key are read from the environment, else from a 0600 file
    in the helper's config folder; each reaches a verb's shell on stdin. A registration token and a git token
    live in memory for one command. None is in an argument, the log or any output. The rules and their enforcers
    are in [README.md](README.md).
15. **The pass, the job, the release.** `shed-cloud tick` is the 5-minute pass; `shed-cloud install` writes its
    job (`dev.shed.cloud`) through farrier-service, as `shed install` writes shed's. Each release carries
    `shed-cloud-<target>` beside `shed-<target>` under one `SHA256SUMS`. `shed self-update` keeps any binary of
    its release that is installed beside `shed` at shed's build, by asset name, so shed's code names no helper.
16. **drover.** It runs `shed-cloud ls --json` to learn which machines are pool VMs, with their tags, slots and
    issues, and `shed-cloud demand --by drover` with what still waits. Its lanes' terminals carry their issue as the `item`
    label, as today, and it places a lane on the VM that has had its issue first. It reads every machine's state from shed. It
    runs no `start`, `stop` or `rm`. Everything else it does to a host is a shed call on a machine
    (drover#1575).
17. **Tests.** Over #308's fake link, with a provider record that is shell lines over the fake link's folders;
    the time is a value the test sets (#310); the machine's states are asserted on shed, where they live. The
    pool VM joins #308's nightly soak in slice 5. This issue builds no fake link, no soak and no clock.
    [README.md](README.md).

What this reverses, said plainly:

- **#259, decision 3 and the VM half of slice H4.** No `vm.*` methods, no `shed-hosts/vm.rs` or `pin.rs`, no
  `vms/` records in shed-catalog, no provider key among shed's own secrets. H4's other half (`machine add
  --install`, `machine.setup`) is not about VMs and stands.
- **#259's split rows "cloud VMs" and "runner VMs".** Budget, spend, idle stop, the start itself, runner
  registration, tokens and caps leave drover for the helper. drover keeps placement, the save of a lane's work,
  its alerts and its board. #259's D6 is replaced by drover#1644.
- **#259's "shed stops nothing by itself".** Still true of shed's server. The helper ends VMs by itself; that
  is its job.
- **#259's C5 (cooee's desks on `vm.*`).** There is no `vm.*` to move to. The owner decided: desks stay in
  cooee as they are.
- **drover's runner approvals.** Today an owner's yes lets one `runners add` pass `max_vms` or `usd_day`. The
  helper knows no asks: a cap is a number in its config.
- **drover waking a stopped cloud host.** A stopped named VM stays stopped until the owner starts it.
- **This ADR's first draft.** drover called `shed-cloud start`, and a `before_stop` hook saved work. Both are
  gone: demand and drain replace them.
- **This ADR's second draft.** Its slice 8 (the fake link, the fault tests, the soak) is #308, which merged
  first. Its `max_lanes` counted terminals. The helper had a `draining` state and three holds of its own; they
  are shed's rows now.
- **"The one core change".** The amendment names `draining` as shed's one core change. There are two: the pinned
  host key (option K) is the other. Without it a new VM's key would be trusted through the owner's own
  `~/.ssh/config`, as drover does today.

## Consequences

- drover and shed's server call no Boat CLI or API. A budget holds while drover is stopped, and while shed's
  server is down: the pass needs only the provider and the log to see that a VM is over.
- shed gains one crate of about 1,800 lines and about 160 lines in the server and CLI; the cap needs one
  raise, its own issue, filed when slice 3 starts.
- The helper's slices wait for #309's: slices 1 and 2 for its tables, engine and runner; slices 3 to 7 also
  for its terminal take-over (the counts by state) and its machine take-over (`machine.ls`), which come after
  the shadow week. Nothing of the helper is built on today's `machine.links`.
- `machine.add` can replace an entry, a machine entry can pin a key, and `machine.drain` exists. That raises
  `PROTOCOL` twice (slices 1 and 2). The helper refuses `SHED_OLD` on a main-host server without `machine.ls`
  and on a VM whose shed is under slice 2's protocol, since an older shed would ignore the drain without
  saying so.
- Nothing writes `~/.ssh/config`. The block in `~/.ssh/config.d/herdsman`, its `Include` and
  `~/.ssh/herdsman/*.known` are no longer needed once drover's issue lands.
- A start trusts the key the record's `address` prints right after it (boat: a key scan of the endpoint Boat's
  API named), as drover's start does today, and logs it. A key that changes while the VM runs is refused by
  shed, and only the owner's `repin` accepts it.
- A pool VM's name is never used twice (`dev-17`, then `dev-18`), so a machine name always means one disk.
- The first lane after a quiet time waits for a VM to be created from the image. The pool keeps no VM up
  without demand.
- An issue's next round finds its VM warm only while that VM lives: a wait longer than the idle limit (an
  owner's answer, a long CI run) ends the VM, and the round starts on another one from the image.
- `max_lanes` is held by drover's placement, from the row's `items`: shed refuses nothing for it. A third issue
  placed in the minutes before the helper's next pass is only shown (`items 3 of 2`); the VM leaves as it
  would have.
- A terminal that never ends, or a finished one nobody removes, keeps a leaving VM up, past its budget too.
  shed's `drain-slow` event after an hour is drover's to alert on. A finished terminal frees its name by
  itself after 24 h (shed's rule). Only the owner's `--now` ends a VM with terminals on it. The provider's own
  time limit (twice `max_life_hours`) is the last stop for a pool VM.
- A machine that is `offline` or `key_changed` cannot be counted, so its VM is not ended and goes on costing
  until the link returns, the owner acts, or the provider's time limit ends it.
- An idle stop no longer needs drover's lane list: any working shed terminal on the VM keeps it up, an owner's
  included. A shell over plain ssh does not, as today.
- A VM made from the image already has shed. When its build differs from the main host's, the helper installs
  the release asset through the record's `exec`, checked against the sum the main host read; the next image
  build then carries the new one.
- A git token is good for an hour at most after its VM is deleted. Nothing revokes it sooner.
- Runner VMs keep their two caps and get no monthly budget, no idle stop and no drain: `runner rm` drains the
  runners themselves. `runners retire-local` and removing a stray runner by name are dropped: neither is about a
  VM ([port-map.md](port-map.md)).
- The provider key stays a 0600 file, as in drover (#707). When #259's `shed-secrets` crate lands, the helper
  opens the same place through it, which on macOS is the Keychain.
- The helper is installed on the main host only, and is optional: without it shed and drover work, with no
  rented machines.

## Hot files

The helper is new code, laid out so its slices share no file: one file per concern in
`crates/shed-cloud/src/`, one per command in `src/cmd/` (collected by `build/collect.rs`, as `crates/shed`'s
are), one per provider in `providers/`, one per git record in `git/`, one docs file per area in `docs/cloud/`,
one gate file. Slices 1 and 2 write in #309's layout: one file per flow in `crates/shed-proto/flows/` and in
`crates/shed-server/src/flows/`, both collected folders of #309's. It converts no existing hot file. It
touches two shared ones: `.drover/arch.toml` (the lines below; the file is at 479 of its 480 lines) and
`.github/workflows/release.yml` (slice 4 adds the second binary).

Slice 3 adds to `.drover/arch.toml`, and raises that file's own `[[size]]` rule by the 65 lines it adds:

```toml
[[layer]]
name = "cloud"
paths = ["crates/shed-cloud/src/**"]
mods = ["shed_cloud"]
may_use = ["core", "proto", "client"]

[[ban]]
pattern = "shed-cloud"
paths = ["crates/*/src/**"]
except = ["crates/shed-cloud/src/**"]
reason = "#307: shed's server and CLI are VM-free; nothing in them names or starts the helper"

[[ban]]
pattern = "boat"
paths = ["crates/*/src/**"]
except = []
reason = "#307: a VM provider is a record in crates/shed-cloud/providers/<name>.toml; code names none"
comments = true
ignore_case = true

[[ban]]
pattern = "shed_client"
paths = ["crates/shed-cloud/src/**"]
except = ["crates/shed-cloud/src/shed.rs"]
reason = "#307: one file asks shed and reads its facts"

[[ban]]
pattern = "flow::"
paths = ["crates/shed-cloud/src/**"]
except = ["crates/shed-cloud/src/shed.rs"]
reason = "#307: a machine's states are shed's table (#309); the helper compares none"

[[ban]]
pattern = "unix_now"
paths = ["crates/shed-cloud/src/**"]
except = ["crates/shed-cloud/src/main.rs"]
reason = "#307: the time is a value (Cx.now), so a test sets it (#310)"

[[collect]]
dir = "crates/shed-cloud/src"
glob = "*.rs"

[[collect]]
dir = "crates/shed-cloud/src/cmd"
glob = "*.rs"

[[collect]]
dir = "crates/shed-cloud/providers"
glob = "*.toml"

[[collect]]
dir = "crates/shed-cloud/git"
glob = "*.toml"

[[collect]]
dir = "docs/cloud"
glob = "*.md"
```

In the same slice two existing bans each gain one `except` entry: `env::var` gains
`"crates/shed-cloud/src/paths.rs"`, and `.expose()` gains `"crates/shed-cloud/src/exec.rs"` (its `reason` gains
"or a provider's key, a runner token and a git token into a verb's stdin"). No other ban changes: the helper
starts processes through `shed_core::runner` and waits through `shed_core::clock`, like every crate.

Slice 3 also adds the gate `.gatekeeper/gates/cloud-free.toml` (a new file in a collected folder):

```toml
about = "#307: shed and the crates it uses do not depend on shed-cloud, and shed-cloud links no server"
fix = "move the VM code into crates/shed-cloud, or the shared code into shed-core"
budget_secs = 20
paths = ["crates/**", "Cargo.lock"]
run = """
set -eu
! cargo tree --locked -p shed -e normal,build | grep -q shed-cloud
! cargo tree --locked -p shed-cloud -e normal,build | grep -Eq 'shed-(server|accounts|catalog|script|search)'
"""
```

Slice 3's tests use the fake link, so `.gatekeeper/gates/fake-link.toml`'s `paths` gain
`"crates/shed-cloud/**"` and its `run` gains `-p shed-cloud`: one gate for every test over the link, as #308
set it up.
