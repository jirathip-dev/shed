# The pool, draining and the golden image

How drover and the owner use these reads first, in [README.md](README.md). The types are in
[shapes.md](shapes.md).

## Demand: what drover publishes

drover places lanes on the machines it knows: its persistent hosts and the running pool VMs that
`shed-cloud ls --json` lists. What it could place nowhere it publishes, once a pass, also when nothing waits:

```console
$ echo '{"waiting":[{"tags":["linux","docker"],"lanes":3},{"tags":["linux","gpu"],"lanes":1}]}' \
    | shed-cloud demand --by drover --json
{"created":["dev-18","dev-19"],"short":[{"tags":["linux","gpu"],"lanes":1,"why":"tag_max","note":"gpu: 1 of 1 VMs"}]}
```

- An entry is "this many lanes wait that need all of these tags". A lane whose issue is warm on a VM with a
  free slot is not in it: drover placed it there.
- A publish replaces that publisher's last one. The helper keeps it in `<state>/shed-cloud/demand/<by>.json`
  with the time it arrived, by its own clock.
- A publish older than 15 minutes (`[caps] demand_ttl_minutes`) counts as nothing: a publisher that stopped
  asks for nothing.
- `demand` sizes the pool at once and answers within a second. Each create runs on as its own detached
  process, the same code as `start`, under the VM's lock; the pass finishes one whose process died.
- `short` says what a cap held back and which cap. drover shows it on the waiting lane.

## Sizing

One pure function, run by `demand` and by every pass:

```rust
pub fn plan(config: &Config, vms: &[PoolVm], demand: &[Want], spent_usd_month: f64) -> Plan;
```

1. Each entry goes to the cheapest pool (the rate of its size, then its name) whose `tags` hold every tag of
   the entry and which has an image. None: `short`, why `no_pool` or `no_image`.
2. The room a pool already has for a new issue: for each of its VMs, `min(slots, max_lanes)` while it is
   `starting` or `setup`; `min(slots - working terminals, max_lanes - issues it has had)` while it is
   `running`; nothing once it is leaving.
3. VMs to create: the lanes left after that room, divided by `min(slots, max_lanes)`, rounded up.
4. Cut to what each cap leaves, in this order: the pool's `max_vms`, `[caps] max_per_tag` for each of the pool's
   tags, `[caps] max_vms`, then the budget (no create at or over `[caps] budget_usd_month`). A leaving VM counts
   against the caps until it is deleted, because it still costs. What is cut is `short` with that cap's name.

Demand creates VMs and never removes one. A VM leaves by its own limits, below.

## A pool VM's life

The statechart is in [README.md](README.md). A pool VM is created from the pool's image, set up, a shed machine
while it runs, told to leave, deleted. It is never stopped and never resumed, so it has one address and one
host key for its whole life.

**Issues, not terminals.** `max_lanes` counts the issues a VM has had: the distinct values of the `item` label
on the terminals shed lists there, taken with the `group` label, so two repos' issue 12 are two. drover's spawn
sets both on every lane today. One issue's impl, its reviews and its fix rounds are one item, however many
terminals they are. A terminal with no `item` (an owner's shell, the image build, a warm spare) is no issue;
it still keeps the VM busy while it works. The helper logs `item` the first time it sees a value on a VM, and
never reads what the value means.

| limit | default | what happens at it |
|---|---|---|
| `max_lanes` | 2 | at this many issues the VM takes no new one. Its own issues' later lanes still go there, by drover's placement. It leaves as soon as shed lists no terminal on it: its issues' lanes were removed, so nothing can return to it |
| `idle_minutes` | 20 | it leaves after this long with no terminal `starting` or `running`, counted from its `up` line |
| `max_life_hours` | 12 | it leaves at this age. The provider's own time limit is set to twice that at create, for a helper that died |
| the budget | `[caps]` | every pool VM leaves when the month's spend reaches it |
| the owner | - | `shed-cloud drain <vm>` |

Leaving is: the helper logs `leave` with the reason, asks shed to drain the machine, and deletes the VM when
shed lists no terminal there, finished ones included. The machine is removed from shed first, then the
provider's `destroy` runs, then `removed` is logged.

`max_lanes` is a rule of placement, which is drover's: the row's `items` says which issues a VM has had, and
drover puts a lane on the VM that has had its issue, else on one under its count. shed refuses nothing for it.

Setup of a pool VM, in order: the record's `ready`; `address`; `machine.add` with the key and the identity
file; shed shows the machine `up`; shed's build checked (installed again only when it differs from the main
host's); the git token; `up`. A step that fails deletes the VM. Two VMs in a row that fail setup from a new
image mark that image bad, and the pool uses the one before it.

## `draining` in shed (slice 2)

Draining is the drain flow of one shed server, rows D1 to D5 of #309's table
([../state-table/tables.md](../state-table/tables.md)), kept in `drain.json` in its state folder so it
outlives a restart and a self-update. It is not VM code: any shed can be drained, the main host's own
included. The main host's machine flow reads it from that shed's stats: `up` to `draining` (M13) and back
(M16).

```console
$ shed machine drain --on dev-16 --why "idle 20 min"
dev-16 draining
$ shed --on dev-16 run --name x -- true
error: DRAINING: dev-16 takes no new terminal since 14:02 (idle 20 min)
$ shed machine stats --on dev-16
load/cpu 0.41  memory 38%  …  terminals done=1,gone=1  draining since 14:02 (idle 20 min)
$ shed machine drain --on dev-16 --off
dev-16 no longer draining
```

| while draining | | row |
|---|---|---|
| refused | `run`, whoever calls it and however it arrives: error `DRAINING` with the time and the reason | the edge, under the lock that creates a terminal |
| ended | the server's warm spares; it starts no new one | D1's `end-spares` |
| unchanged | every call on a terminal that exists (`send`, `keys`, `read`, `wait`, `kill`, `rm`, `attach`), `exec`, `file.*`, `ls`, `events`, `machine.stats`, the token-proxy forward, a self-update | - |
| shown | `machine.stats`: `draining: { since, why }`; `machine.ls`: the machine's `flow` is `draining`; the refusal names both | M13 |
| slow | still draining after an hour: one `stuck: drain-slow` event, and the machine's `flow` shows it | M13's 3600 s, M18 |
| lifted | `machine.drain` with `on: false`. Draining twice changes nothing and keeps the first time | D2, D4; M16 |

How each program uses it:

- **The helper** logs `leave`, then calls `machine.drain`. It ends the VM only on a stats answer that says
  `draining` and lists no terminal in any state. The server on the VM refuses `run` under the same lock that
  creates a terminal, so zero cannot become one. A pass that finds a leaving VM not draining (lifted by hand,
  or the first call lost) asks again. After a named VM starts again the helper lifts the drain its disk still
  carries.
- **drover** reads `draining` from `machine.ls`, as for any machine, and places nothing there. A `run` that
  loses the race gets `DRAINING` and is placed again next pass. When a lane ends on a draining machine drover
  saves its unpushed work to `wip/`, then removes the lane's terminal. It routes `drain-slow`.
- **The rule that makes the save safe:** a finished terminal still counts. Its name is `free` only after its
  owner's `rm` (rows T27, T34), which drover does after the save. A helper that looked only for working
  terminals could delete the VM between a lane's exit and drover's save.
- **The owner** can drain a persistent host by hand before maintenance. drover treats it the same way.

A warm spare is no terminal in these counts. Slice 2 makes `machine.stats` count it as `spare` (today it
counts as `running`), so a spare neither keeps a VM busy nor holds it from zero.

A leaving VM whose machine does not answer is not ended: its terminals cannot be counted, and shed's machine
flow shows it `offline`. A VM the provider no longer runs is logged `removed`, why `outside`, whatever it held.

In the helper the end takes a proof, so no path can skip the rule:

```rust
pub struct Drained(());                                        // in leave.rs; built only by the three functions below
pub fn zero(seen: &Counts) -> Option<Drained>;                 // shed says it is draining and lists no terminal
pub fn gone(seen: Seen) -> Option<Drained>;                    // the provider no longer runs it
pub fn owner_now() -> Drained;                                 // `--now`, typed by the owner
pub fn end(cx: &Cx, vm: &Vm, proof: Drained) -> Result<()>;   // in life.rs: `machine.rm`, then stop or destroy as its `leave` line says
```

## Persistent hosts beside pool VMs

A persistent host is a shed machine the owner added with `shed machine add`. The helper does not know it, does
not count it in a cap and never drains it. drover lists it in its own `[hosts]`, as now.

| | persistent host | pool VM | named VM |
|---|---|---|---|
| who adds the shed machine | the owner | the helper, at each create | the helper, at each start |
| drover learns of it from | its `[hosts]` | `ls --json`: a `pool` row that is `running` and not leaving, with the pool's tags and slots and its `items` | its `[hosts]` |
| who starts it | nobody | the helper, from demand | the owner, and nobody else |
| how it ends | it does not | drained, deleted | drained, stopped; resumes with a new address |
| budget | none | `[caps]` | its own |

drover fills the VM that is warm for a lane's issue first, then persistent hosts, since they cost nothing
more, then other running pool VMs, and publishes the rest. That order is drover's (drover#1644); the helper
only sees what is left. A lane that needs a stopped named VM waits for the owner: demand never starts one.

## The golden image

One saved snapshot per pool. A pool VM is created from it and is ready in about the time the provider takes to
copy it.

```toml
[pool.dev.image]
build = "images/dev.sh"                  # in the helper's config folder; run on the build VM
key = ["drover", "image-key", "app"]     # optional: a command on the main host; drover's to name
```

**The key.** The SHA-256 of: the build script's bytes, the `key` command's output, shed's build, the pool's
provider and size. The `key` command is where "toolchains or lockfiles changed" comes in: drover's prints the
hashes of the fleet repos' lockfiles and toolchain files at their integration heads. The snapshot is named
`<pool>-<the key's first 12 hex digits>`. The pass works the key out at most every 30 minutes; a key with no
`image` line in the log starts a build. `shed-cloud image build <pool>` starts one by hand.

**The build**, one at a time per pool, counted in the budget but not in `max_vms`:

1. A fresh VM, `<pool>-image-<n>`, created with no account secrets (boat: `--no-env`), set up like any VM, its
   shed machine added. drover does not place on it: it is not a `pool` row.
2. The VM gets a git token, as every VM does (below), so the script can clone private repos.
3. The build script runs in a shed terminal on the VM, so it outlives a dropped link and the owner can read
   it. Two hours at most. A non-zero exit fails the build.
4. The helper removes the git token and its own terminal, then checks the VM through `exec`: none of the
   record's `never` paths may exist. One that does fails the build with `IMAGE_DIRTY` and its path.
5. The record's `snapshot` saves it under the key's name. The log gets `image`.
6. The build VM is deleted. `snapshot_rm` removes every snapshot of the pool but the newest two.

A failed build logs `image-failed` with the step, deletes the VM, and is tried again when the key next changes
or by hand. Until a build passes the pool creates from the last good image; with none it creates nothing and
`short` says `no_image`. Running VMs are left alone when a new image lands: they leave by their own limits.

| in the image | never in the image | held by |
|---|---|---|
| the fleet's repos, cloned | a provider's key or its account's stored secrets | the create line (`--no-env`); the helper sends no key to a VM |
| toolchains and tools | a GitHub login, a git credential, a `gh` token | removed in step 4; the `never` paths; it expires within the hour anyway |
| shed, at the main host's build | an agent's login or credential file | no agent logs in on a VM (below); the `never` paths |
| a warm dependency build | a token-proxy key, an ssh private key, a secret file a lane's start wrote | the `never` paths; proxy keys are per run and die with it |

The `never` list is data in the provider's record, with the paths of the logins the fleet's tools write, so
the helper's code names no tool.

## Sign-in and the git token

**Agents.** Nothing is installed or logged in on a VM for them. shed's server on the main host forwards its
token proxy over the held link (#265), and a `run` on the machine gets the relay's address and a key for that
run only. That works for a pool VM with no step of the helper's, because the VM is an ordinary machine with a
link from the main host. drover's check that the main host has a token to proxy stays drover's.

**Git.** One token per VM, minted by a GitHub App on the main host, never stored there (the owner's decision:
one new secret, the App's key; pushes and PRs from a VM show the App as their author):

```toml
[git]
record = "github-app"                    # crates/shed-cloud/git/github-app.toml, or the owner's file of that name
app = "123456"
installation = "7654321"
repos = ["owner/app", "owner/lib"]
refresh_minutes = 45
```

| step | where | what |
|---|---|---|
| mint | the main host | the record's `mint` line: the App's key on stdin (a secret, like a provider's key), a token for `repos` only, good for one hour, printed with its expiry |
| install | the VM, through shed's `exec` | the record's `install` line: the token on stdin; it makes git and `gh` there use it |
| refresh | the pass | a new token for every running VM whose last one is older than `refresh_minutes`; logged `git-token` with no value |
| end | - | the token dies within the hour after the VM is deleted |

Without `[git]` a VM gets no git credential.
