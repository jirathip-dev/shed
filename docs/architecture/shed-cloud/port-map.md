# Port map: drover's `cloud.rs` and `runners.rs` (staging `7c0a28c1c`)

Every function, in file order. "Helper" paths are under `crates/shed-cloud/src/`. "Stays" means it stays in
drover, in another module, for drover#1644 to place; "dropped" means no program has it afterwards.

## `crates/core/src/cloud.rs` (669 lines)

| function | new home | notes |
|---|---|---|
| `recipe` | `record.rs` `Record::load`, `config.rs` | the size is checked against the record's rates at load. Dropped: the `medium` → `default` alias; the import writes `default` once |
| `key` | `secret.rs` | the environment, else the 0600 file, now in the helper's own `secrets/` folder |
| `load`, `user_recipe` | `record.rs` | the owner's file is `<config>/shed-cloud/providers/<name>.toml` |
| `builtin` | the crate's `build.rs` | collects `providers/*.toml`, as shed-catalog collects its records |
| `stale_override` | `cmd/doctor.rs` | widened: an owner's record that lacks any key the built-in of that name has |
| `cmd` | `record.rs` `Record::verb` | fills the template; the empty-id refusal becomes `NO_VM` for every verb that uses `{id}` |
| `run` | `exec.rs` | the key and the token on stdin, both cut out of the stderr it returns |
| `log`, `last` | `log.rs` | typed `Event`s in `log.jsonl` instead of drover's event log |
| `state` | `life.rs` `state` | pure; gains `setup` and `absent`. It is the provider's side only: the machine's states are shed's rows (#309) |
| `spend` | `spend.rs` `spent` | pure; the same spans, month and day |
| `over` | `spend.rs` `over` | the same two limits and the same sentences |
| `held` | `ls --json`: a row's `state`, `leaving` and `over`, a pool's `short` | drover shows them on its board and its waiting lanes; it starts nothing on them. A machine's own trouble (offline, key changed, a slow drain) it reads from shed, as for any machine |
| `start` | `life.rs` `start` | run by the owner for a named VM and by the helper, from demand, for a pool VM. drover's call is deleted. Stays: the `NO_PROXY_TOKEN` check (drover's, about its own proxy), before it places a lane |
| `catch_up` | dropped from the helper; stays | it installs drover's own release on a host that just came up: drover's self-update, for any machine whose link returns |
| `outside` | `tick.rs`, `life.rs` `stopped` | also run before a `start` |
| `pin` | gone | the pin is `host_key` in shed's machine entry (slice 1); `~/.ssh/herdsman/` is no longer written |
| `moved` | `tick.rs` → `address.rs` `keep` | every pass, for each running VM |
| `refresh` | `address.rs`, `shed.rs` `add` | parses `address`; another address with the same key is `machine.add` again (row M8). A changed key is refused by shed's own link (row M10), not judged here. Dropped: the block in `~/.ssh/config.d/herdsman` and the known-hosts file |
| `stop` | `leave.rs` `decide`, then `life.rs` `end` | the `leave` line and `machine.drain` first; the verb, `machine.rm` and the log line at zero terminals. Stays: the save of a lane's work to `wip/`, now when a lane ends on a draining machine and followed by the terminal's removal. Dropped: `cloud-abort`; shed's `drain-slow` event says the same |
| `Live`, `tick` | `tick.rs`, `idle.rs`, `limits.rs` | "no live agent" becomes `machine.stats`: no terminal `starting` or `running`. An idle VM is told to leave, drained by shed, then ended. drover's `check` stops calling it |
| `card` | stays | the board's Host card is drover's screen; its numbers come from `ls --json` |
| `github`, `token` | stays, for persistent hosts | the expiry of a host's `gh` token is a host health row. A VM has no such token: `git.rs` gives it one that lives an hour |
| `SH`, `TOOLS`, `ACP`, `RUST` | stay | bootstrap's step lines, for a persistent host. For a pool they become the image's build script, which drover writes (drover#1644) |
| `SHED` and bootstrap's `shed` step | `setup.rs`, for a VM | a VM must run shed before it is a machine, so the helper does this one step, through the record's `exec`; a VM from an image already has it and gets it again only when the build differs. For a machine that is not a VM it stays in drover until #259's `machine add --install` |
| `bootstrap` | stays, for persistent hosts | it bootstraps a remote host, through shed `exec` after drover#1575. Its first lines (start a stopped cloud host, re-pin a running one) are deleted: drover starts nothing, and the pass keeps the pin |
| `snapshot` | `life.rs` `snapshot`; `image.rs` | a named VM: the verb, the log line, and the wait for the link afterwards (the shed step again when it does not return). A pool: the golden image's build. Dropped: the health look before it, with drover's `hosts snapshot` |
| `doctor` | `cmd/doctor.rs` | the key's place, the identity file, the record's CLI subcommands against `--help` (the CLI's name is now the record's `cli` key). Dropped: the old-size row, with the alias |
| `includes` | dropped | it flags an `Include` of a `.known` file under `~/.ssh/config.d`; nothing writes there any more |

## `crates/core/src/runners.rs` (359 lines)

| function | new home | notes |
|---|---|---|
| `Verb` | `cmd/runner.rs` | `add`, `rm`, `revive`, `tokens`; `Status` is the group rows of `ls`. Dropped: `RetireLocal` |
| `Gh` | `runner.rs` `Registered` | a struct with named fields instead of a tuple |
| `conf` | `config.rs` | `[runners.<name>]`; more than one group is allowed |
| `log` | `log.rs` | |
| `vms` | `log.rs` `group_vms` | `runner-add` less `runner-rm`. Dropped: the `vms` config key; the import writes a line per entry |
| `github` | `runner.rs` `registered` | `gh api` on the main host, by label |
| `deregister` | `runner.rs` | |
| `token` | `runner.rs` `registration_token` | returns a `Secret` |
| `rc` | `record.rs` `Record::verb`, `Record::inside` | the `runner_*` keys are the `boat-api` record's verbs and its `[runner]` lines |
| `per_day` | `spend.rs` | the record's rate × 24 |
| `approved` | dropped from the helper; stays | an owner ask is drover's. The helper's caps are hard, and `--last` is the one confirmation it takes; drover checks its ask before it passes `--last` or proposes a cap edit |
| `ask_owner` | stays | drover turns exit 3 `RUNNERS_CAP` and `RUNNERS_LAST` into its ask |
| `health` | `runner.rs` `health` → `ls --json` `online` | Stays: the `runners-low` and `runners-ok` alerts, read from the group's row |
| `push_tokens` | `runner.rs` | |
| `tick` | `tick.rs` → `runner.rs` `pass` | the refresh interval is the group's `token_refresh_minutes` instead of drover's `[limits]` |
| `run`: `Status` | `cmd/ls.rs` | the group's row and its VMs' rows; each VM's provider state is read by the pass, not by `ls` |
| `run`: `Revive` | `cmd/runner.rs` `revive` | |
| `run`: `Add` | `cmd/runner.rs` `add` | the cap refuses; no approval passes it |
| `run`: `Remove` of a VM | `cmd/runner.rs` `rm` | drain, 30 minutes, deregister, destroy; `--last` for the last VM with online runners |
| `run`: `Remove` of a runner by name | dropped | a stray registration is not a VM; `gh api -X DELETE` removes one |
| `run`: `Tokens` | `cmd/runner.rs` `tokens` | |
| `run`: `RetireLocal` | dropped | it stops the main host's own runners and deregisters them: not a VM, and a one-time move (drover#537). drover keeps it or deletes it in drover#1644 |

## What drover calls afterwards

| drover today | after |
|---|---|
| `check.rs`: `cloud::tick`, `runners::tick` | deleted; the helper's own job runs the pass |
| `check.rs`, `hosts.rs`, `update.rs`: `cloud::state(..) == "stopped"` | the row's `state` from `shed-cloud ls --json` |
| `hosts.rs` `at`: `cloud::held` | the row's `state`, `leaving` and `over`; a pool's `short`; the machine's `flow` from shed |
| `hosts.rs` `wake`: `cloud::start` | deleted; `shed-cloud demand --by drover` publishes what waits, every pass |
| `hosts.rs`: the host list | `[hosts]` and each running `pool` row of `ls --json`, with its pool's tags and slots |
| placement | only on a machine whose `flow` in `machine.ls` is `up`; a lane goes to the pool VM whose `items` has its issue, else to one under `max_lanes`; a `run` refused `DRAINING` is placed again next pass |
| a lane's end | on a draining machine: the save to `wip/`, then `rm` of its terminal |
| `hosts.rs` survey: `cloud::moved` | deleted; the pass keeps the machine's address |
| `cli.rs`: `host destroy`, `hosts snapshot` | deleted; the owner runs `shed-cloud rm <vm> --yes` and `shed-cloud snapshot <vm>` |
| `cli.rs`: `hosts bootstrap` | drover's own, minus its start |
| `cli.rs`: `drover runners <verb>` | `shed-cloud runner <verb>`, or deleted, as drover#1644 decides |
| `doctor.rs`: `cloud::doctor`, `cloud::includes` | `shed-cloud doctor`'s rows; `includes` gone |
| `serve.rs`: `cloud::card` | drover's card over the row |
