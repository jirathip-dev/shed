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

- An entry is "this many lanes wait that need all of these tags".
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
2. The room a pool already has: for each of its VMs, `min(slots, max_lanes)` while it is `starting` or `setup`;
   `min(slots - live terminals, max_lanes - lanes it has had)` while it is `running`; nothing while it drains.
3. VMs to create: the lanes left after that room, divided by `min(slots, max_lanes)`, rounded up.
4. Cut to what each cap leaves, in this order: the pool's `max_vms`, `[caps] max_per_tag` for each of the pool's
   tags, `[caps] max_vms`, then the budget (no create at or over `[caps] budget_usd_month`). A draining VM counts
   against the caps until it is deleted, because it still costs. What is cut is `short` with that cap's name.

Demand creates VMs and never removes one. A VM leaves by its own limits, below.

## A pool VM's life

The statechart is in [README.md](README.md). A pool VM is created from the pool's image, set up, a shed machine
while it runs, drained, deleted. It is never stopped and never resumed, so it has one address and one host key
for its whole life.

| limit | default | what happens at it |
|---|---|---|
| `max_lanes` | 2 | the VM drains when its second lane starts. A lane is a terminal shed lists on the machine, counted once by name; warm spares are not counted |
| `idle_minutes` | 20 | the VM drains after this long with no terminal outside `exited`, counted from its `up` line |
| `max_life_hours` | 12 | the VM drains at this age. The provider's own time limit is set to twice that at create, for a helper that died |
| the budget | `[caps]` | every pool VM drains when the month's spend reaches it |
| the owner | - | `shed-cloud drain <vm>` |

Draining ends in a delete when the machine's stats list no terminal, exited ones included. The machine is
removed from shed first, then the provider's `destroy` runs, then `removed` is logged.

Setup of a pool VM, in order: the record's `ready`; `address` and the pin; `machine.add`; the link up; shed's
build checked (installed again only when it differs from the main host's); the git token; `up`. A step that
fails deletes the VM. Two VMs in a row that fail setup from a new image mark that image bad, and the pool uses
the one before it.

## `draining` in shed (slice C2)

Draining is a state of one shed server, held by that server, kept in its state folder so it outlives a restart
and a self-update. It is not VM code: any shed can be drained, the main host's own included.

```console
$ shed machine drain --on dev-16 --why "lanes 2 of 2"
dev-16 draining
$ shed --on dev-16 run --name x -- true
error: DRAINING: dev-16 takes no new terminal since 14:02 (lanes 2 of 2)
$ shed machine stats --on dev-16
load/cpu 0.41  memory 38%  …  terminals running=1,exited=1  draining since 14:02 (lanes 2 of 2)
$ shed machine drain --on dev-16 --off
dev-16 no longer draining
```

| while draining | |
|---|---|
| refused | `run`, whoever calls it and however it arrives: error `DRAINING` with the time and the reason |
| ended | the server's warm spares; it starts no new one |
| unchanged | every call on a terminal that exists (`send`, `keys`, `read`, `wait`, `kill`, `rm`, `attach`), `exec`, `file.*`, `ls`, `events`, `machine.stats`, the token-proxy forward, a self-update |
| shown | `machine.stats`: `draining: { since, why }`; `shed machine stats` prints it; the refusal names both |
| lifted | `machine.drain` with `on: false`. Draining twice changes nothing and keeps the first time |

How each program uses it:

- **The helper** calls `machine.drain` on a VM before every stop and every delete, and logs `drain`. It ends
  the VM only on a stats answer read after that call which lists zero terminals. The server on the VM refuses
  `run` under the same lock that creates a terminal, so zero cannot become one. After a named VM starts again
  the helper lifts the drain its disk still carries.
- **drover** reads `draining` in the `machine.stats` answer it already takes for placement, and places nothing
  on that machine. A `run` that loses the race gets `DRAINING` and is placed again next pass. When a lane ends
  on a draining machine drover saves its unpushed work to `wip/`, then removes the lane's terminal.
- **The rule that makes the save safe:** an exited terminal still counts. The VM is deleted only after its
  owner removed it, which drover does after the save. A helper that looked only for live terminals could delete
  the VM between a lane's exit and drover's save.
- **The owner** can drain a persistent host by hand before maintenance. drover treats it the same way.

A draining VM whose link is down is not ended: its terminals cannot be counted. After 15 minutes the row holds
`unreachable`. A VM the provider no longer runs is logged `removed`, why `outside`, whatever it held.

In the helper the end takes a proof, so no path can skip the rule:

```rust
pub struct Drained(());                                        // in drain.rs; built only by the three functions below
pub fn zero(stats: &MachineStats, drained_at: u64) -> Option<Drained>;   // draining since before the sample, no terminal
pub fn gone(seen: Seen) -> Option<Drained>;                    // the provider no longer runs it
pub fn owner_now() -> Drained;                                 // `--now`, typed by the owner
pub fn end(cx: &Cx, vm: &Vm, proof: Drained) -> Result<()>;   // in life.rs: `machine.rm`, then stop or destroy as its `drain` line says
```

## Persistent hosts beside pool VMs

A persistent host is a shed machine the owner added with `shed machine add`. The helper does not know it, does
not count it in a cap and never drains it. drover lists it in its own `[hosts]`, as now.

| | persistent host | pool VM | named VM |
|---|---|---|---|
| who adds the shed machine | the owner | the helper, at each create | the helper, at each start |
| drover learns of it from | its `[hosts]` | `ls --json`: a `pool` row that is `running`, with the pool's tags and slots | its `[hosts]` |
| who starts it | nobody | the helper, from demand | the owner |
| how it ends | it does not | drained, deleted | drained, stopped; resumes with a new address |
| budget | none | `[caps]` | its own |

drover fills persistent hosts first, since they cost nothing more, then running pool VMs, and publishes the
rest. That order is drover's (drover#1644); the helper only sees what is left.

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

**Git.** One token per VM, minted on the main host, never stored there:

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

Without `[git]` a VM gets no git credential. With another record the same steps carry another kind of token:
that is the fallback of open question 1, and it changes no code.
