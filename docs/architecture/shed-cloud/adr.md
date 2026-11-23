# ADR: shed-cloud, an optional helper binary that rents VMs; shed's server stays VM-free (#307)

## Summary

Decision: `shed-cloud` is a new crate and binary in shed's repo that nothing else in shed depends on. It rents VMs from one data file per provider (Boat today), keeps each VM's monthly budget, and by itself, every 5 minutes on the main host, stops a VM that is over budget or has had no live terminal for its idle limit. A lane-host VM is a shed machine exactly while it runs: the helper adds it with its address and pinned host key at each start and removes it at each stop. A CI runner VM is never a shed machine; its setup step registers GitHub runners. drover calls two commands, list and start, and otherwise sees only shed machines.
Slices: (1) shed's machine list takes a port, a pinned host key and an identity file; (2) the helper: provider records, start, stop, remove, snapshot, spend and budget, list, doctor; (3) the 5-minute pass, idle stop, its job, its release and the import from drover; (4) runner pools. Then drover's own issues (drover#1644). About 1,400 lines added in shed, about 850 leave drover.
Open questions: (1) #259 put cooee's desks on VM methods in shed's server, which this ADR removes: desks stay in cooee as they are now (recommended), or become a second provider of the helper later? (2) one cap raise of about 1,000 lines in its own cap issue, sized by `drover cap berth` when filed.

**Status:** proposed by the architect lane, 2026-11-23, read on shed `a3a603b` and drover staging `7c0a28c1c`.
It follows the owner's decision of the same day (drover#1644 and its amendment: cloud VMs leave drover; the
helper lives in shed's repo as its own optional binary; shed's server core stays VM-free). How the callers read,
the statecharts, the CLI, the boundaries, the secrets and the slices: [README.md](README.md). Config, provider
record, log and wire shapes: [shapes.md](shapes.md). Every function of drover's `cloud.rs` and `runners.rs`:
[port-map.md](port-map.md).

## The question

drover rents VMs itself, in two files: `cloud.rs` (669 lines: recipes and keys, create and resume, stop, the VM's
changing address and host key, bootstrap and snapshot, spend and the budget hold, the idle-stop pass, doctor rows)
and `runners.rs` (359 lines: runner VMs, registration tokens, health). drover#1644 counts 15 dependencies on
`cloud` inside drover (spawn, hosts, check, update, doctor, the board). A resumed Boat VM comes back with a new address and a new host
key; drover#1644 records that stop and resume broke lanes on 2026-11-23, and the owner's fleet keeps its hosts up
600 minutes to avoid a resume.

The accepted hosts ADR (#259, one day older) moved the VM mechanism into shed's server as `vm.*` methods and
left budgets, idle stop and runner registration in drover. The owner has since decided otherwise: one optional
helper owns all of it, and shed's server knows machines only. So: where does the helper live and what may it
use, what does it keep on disk, how is a provider described, how does a VM that moves stay a shed machine, and
what is left for drover to call?

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
| H | **`shed-client`, as cooee and drover do** | chosen: typed calls, the protocol handshake, the client's fake server in tests |

**How a VM's address and host key reach shed**

| | option | verdict |
|---|---|---|
| I | The helper writes a block in `~/.ssh/config.d/` and shed keeps an alias (drover today) | needs an `Include` the owner adds by hand; the address lives in a second place; #259 already retired it |
| J | shed dials whatever the helper tells it per call | every caller of the machine would need the helper |
| K | **shed's machine entry holds the target with its port, the pinned key and the identity file; the helper replaces the entry when they change** | chosen: three ssh facts any machine can have, about 90 lines in the server, nothing about VMs |

**Where state lives**

| | option | verdict |
|---|---|---|
| L | A state file with counters (seconds this month, last busy) | a crash between a provider call and the write leaves it wrong; no record of why a VM stopped |
| M | **An append-only log; state, spend and idle are functions of it** | chosen: as drover does today, so this month's spend imports exactly, and each rule is tested as a table |

**How idle is known**

| | option | verdict |
|---|---|---|
| N | drover tells the helper which hosts have lanes | the helper would depend on drover running |
| O | **`machine.stats`: no terminal on the machine in a state other than `exited`** | chosen: a lane is a shed terminal, and so is anything else worth keeping a VM up for |

**The work on a VM before it stops (drover's wip save)**

| | option | verdict |
|---|---|---|
| P | drover polls the helper and stops VMs itself | the budget would hold only while drover runs |
| Q | **A `before_stop` command per VM in the helper's config; a non-zero exit aborts the stop** | chosen: the helper names no lane or worktree, drover sets it to its save command |

**Is a runner VM a shed machine?**

| | option | verdict |
|---|---|---|
| R | Yes, like a lane host | shed would be installed on a VM that runs pull-request code, with a held link from the main host that can carry the token-proxy forward (#265) |
| S | **No: it is reached only through the provider's `exec`, and nothing of ours is on it** | chosen: as today, and as drover#1644 words it ("a shed machine or a GitHub runner") |

## The decision

1. **Where it lives.** Crate `crates/shed-cloud`, binary `shed-cloud`. It uses `shed-core` (to start
   processes and read the clock), `shed-proto` and `shed-client`, and the workspace's `clap`, `serde`,
   `serde_json`, `toml`, `anyhow` and `farrier-service`. No new dependency. No crate uses it. Three checks hold
   that: an `arch.toml` layer no other layer lists, a ban on its name in every other crate's source, and a gate
   that fails when `cargo tree -p shed` names it or `cargo tree -p shed-cloud` names the server.
2. **Providers.** A provider is a record, `crates/shed-cloud/providers/<name>.toml`, built in; the owner's file
   of the same name in the helper's config folder replaces it. Verbs: `create`, `start`, `stop`, `state`,
   `destroy`, and where it has them `id`, `address`, `ready`, `snapshot`, `create_from`, `exec`. Two records
   ship: `boat` (the CLI, for lane hosts) and `boat-api` (the HTTP API, for runner VMs), both from drover's
   recipe. The contract is in [shapes.md](shapes.md). The code names no provider.
3. **Config and state.** The owner's `config.toml` lists each lane-host VM (`[vm.<name>]`: provider, size,
   monthly budget, hours a day, idle limit, `before_stop`) and each runner pool (`[runners.<pool>]`). The helper
   writes one file, `log.jsonl`. A VM's state, its spend this month, its hours today and how long it has been
   quiet are computed from the log and the config each time.
4. **Budget.** Spend is the time between a `start` line and the next `stop` line, within the UTC month, times
   the record's rate for the VM's size. A VM with no `budget_usd_month` does not load. `start` refuses
   `OVER_BUDGET` at the month's budget or the day's hours; the pass stops a running VM that reaches either.
5. **Idle stop.** Each pass asks `machine.stats` for the machine, with the samples since the last pass. No
   terminal outside `exited` in any of them logs `quiet` once; one logs `busy`. A VM quiet for its
   `idle_stop_minutes` (default 20) is stopped. A machine that does not answer is not quiet.
6. **The machine link.** After a start the helper reads the record's `address`, pins the key it prints and calls
   `machine.add` with the target `ssh://user@host:port`, the key and the identity file. Each pass reads `address`
   again: a moved address replaces the machine's entry; a changed key replaces nothing and holds the VM as
   `key_changed` until the owner runs `repin`. A stop, from anyone, removes the machine. So shed never dials an
   address a stopped VM gave back.
7. **Runner VMs.** A pool's VM is created, then set up: a registration token from `gh` on the main host, sent on
   stdin through the record's `exec`, and the runner service restarted. The pass sends a fresh token every 30
   minutes and reads how many of the VM's runners GitHub lists online. Revive, add and remove are commands.
   `max_vms` and `usd_day` are hard: only an edit of `config.toml` raises them.
8. **Secrets.** A provider's key is read from the environment, else from a 0600 file in the helper's config
   folder; it reaches a verb's shell on stdin. A registration token lives in memory for one command. Neither is
   in an argument, the log or any output. The rules and their enforcers are in [README.md](README.md).
9. **The pass, the job, the release.** `shed-cloud tick` is the 5-minute pass; `shed-cloud install` writes its
   job (`dev.shed.cloud`) through farrier-service, as `shed install` writes shed's. Each release carries
   `shed-cloud-<target>` beside `shed-<target>` under one `SHA256SUMS`. `shed self-update` keeps any binary of
   its release that is installed beside `shed` at shed's build, by asset name, so shed's code names no helper.
10. **drover.** It runs `shed-cloud ls --json` to learn which of its machines are rented, what state each is in
    and what holds it, and `shed-cloud start <vm>` when placement wants a stopped one. Its board and alerts read
    the same rows. Everything else it does to a host is a shed call on a machine (drover#1575).

What this reverses, said plainly:

- **#259, decision 3 and the VM half of slice H4.** No `vm.*` methods, no `shed-hosts/vm.rs` or `pin.rs`, no
  `vms/` records in shed-catalog, no provider key among shed's own secrets. H4's other half (`machine add
  --install`, `machine.setup`) is not about VMs and stands.
- **#259's split rows "cloud VMs" and "runner VMs".** Budget, spend, idle stop, the start itself, runner
  registration, tokens and caps leave drover for the helper. drover keeps placement (when to ask for a start),
  the save command it puts in `before_stop`, its alerts and its board. #259's D6 is replaced by drover#1644.
- **#259's "shed stops nothing by itself".** Still true of shed's server. The helper stops VMs by itself; that
  is its job.
- **#259's C5 (cooee's desks on `vm.*`).** There is no `vm.*` to move to: open question 1.
- **drover's runner approvals.** Today an owner's yes lets one `runners add` pass `max_vms` or `usd_day`. The
  helper knows no asks: a cap is a number in its config.

## Consequences

- drover and shed's server call no Boat CLI or API. A budget holds while drover is stopped, and while shed's
  server is down: the pass needs only the provider and the log to stop a VM.
- shed gains one crate of about 1,300 lines and about 90 lines in the server; the cap needs a raise (question 2).
- The server's machine entry gains a pinned key and an identity file, and `machine.add` can replace an entry.
  That raises `PROTOCOL` by one; the helper refuses `SHED_OLD` on a server under it, since an older server
  would ignore the pin without saying so.
- Nothing writes `~/.ssh/config`. The block in `~/.ssh/config.d/herdsman`, its `Include` and
  `~/.ssh/herdsman/*.known` are no longer needed once drover's issue lands.
- A start trusts the key the record's `address` prints right after it (boat: a key scan of the endpoint Boat's
  API named), as drover's start does today, and logs the old and new key as `repin`. A key that changes while
  the VM runs is refused.
- A stopped VM is not in `machine ls`. drover tells "stopped" from "offline" by `shed-cloud ls`.
- A `before_stop` that keeps failing keeps a VM up past its budget, as drover's `cloud-abort` does today. The
  row shows `stop_aborted` with the reason, and drover alerts on it.
- An idle stop no longer needs drover's lane list: any live shed terminal on the VM keeps it up, an owner's
  included. A shell over plain ssh does not, as today.
- A fresh VM gets shed through the record's `exec`: the release asset for its OS and CPU, checked against the
  sum the main host read. When #259's `machine add --install` lands, that one step calls it instead.
- Runner VMs keep their two caps and get no monthly budget and no idle stop. `runners retire-local` and
  removing a stray runner by name are dropped: neither is about a VM ([port-map.md](port-map.md)).
- The provider key stays a 0600 file, as in drover (#707). When #259's `shed-secrets` crate lands, the helper
  opens the same place through it, which on macOS is the Keychain.
- The helper is installed on the main host only, and is optional: without it shed and drover work, with no
  rented machines.

## Hot files

The helper is new code, laid out so its four slices share no file: one file per concern in
`crates/shed-cloud/src/`, one per command in `src/cmd/` (collected by `build/collect.rs`, as `crates/shed`'s
are), one per provider in `providers/`, one docs file per area in `docs/cloud/`, one gate file. It converts no
existing hot file. It touches two shared ones: `.drover/arch.toml` (the lines below; the file is at 479 of its
480 lines) and `.github/workflows/release.yml` (slice 3: build, sign and publish the second asset).

Slice 2 adds to `.drover/arch.toml`, and raises that file's own `[[size]]` rule from 480 to 540:

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
dir = "docs/cloud"
glob = "*.md"
```

In the same slice two existing bans each gain one `except` entry: `env::var` gains
`"crates/shed-cloud/src/paths.rs"`, and `.expose()` gains `"crates/shed-cloud/src/exec.rs"` (its `reason` gains
"or a provider's key and a runner token into a verb's stdin"). No other ban changes: the helper starts processes
through `shed_core::runner` and reads the time through `shed_core::clock`, like every crate.

Slice 2 also adds the gate `.gatekeeper/gates/cloud-free.toml` (a new file in a collected folder):

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
