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
| `state` | `life.rs` `state` | pure; gains `setup` and `absent` |
| `spend` | `spend.rs` `spent` | pure; the same spans, month and day |
| `over` | `spend.rs` `over` | the same two limits and the same sentences |
| `held` | `ls --json`: `state` and `hold` | drover maps them to its `CLOUD_STOPPED` and `CLOUD_BUDGET`, which stay its names |
| `start` | `life.rs` `start` | Stays: the `NO_PROXY_TOKEN` check before it (drover's, about its own proxy) and the `hosts::survey` after it |
| `catch_up` | dropped from the helper; stays | it installs drover's own release on a host that just came up: drover's self-update, for any machine whose link returns |
| `outside` | `tick.rs`, `life.rs` `stopped` | also run before a `start` |
| `pin` | gone | the pin is `host_key` in shed's machine entry (slice C1); `~/.ssh/herdsman/` is no longer written |
| `moved` | `tick.rs` → `link.rs` `keep` | every pass, for each running VM |
| `refresh` | `link.rs` | parses `address`, compares the key, calls `machine.add`. Dropped: the block in `~/.ssh/config.d/herdsman` and the known-hosts file |
| `stop` | `life.rs` `stop` | the verb, `machine.rm`, the log line. Stays: the loop that saves each lane's work to `wip/`, as the command drover puts in `before_stop`; its `cloud-abort` is the helper's `stop_aborted` hold |
| `Live`, `tick` | `tick.rs`, `idle.rs` | "no live agent" becomes `machine.stats`: no terminal outside `exited`. drover's `check` stops calling it |
| `card` | stays | the board's Host card is drover's screen; its numbers come from `ls --json` |
| `github`, `token` | stays | the expiry of a host's `gh` token is a host health row for any machine, not a VM fact |
| `SH`, `TOOLS`, `ACP`, `RUST` | stay | bootstrap's step lines |
| `SHED` and bootstrap's `shed` step | `setup.rs`, for a VM | a VM must run shed before it is a machine, so the helper does this one step, through the record's `exec`. For a machine that is not a VM it stays in drover until #259's `machine add --install` |
| `bootstrap` | stays | it bootstraps any remote host, through shed `exec` after drover#1575. Its first lines (start a stopped cloud host, re-pin a running one) become `shed-cloud start <vm>` and `shed-cloud repin <vm>` |
| `snapshot` | `life.rs` `snapshot` | the verb, the log line, and the wait for the link afterwards (the shed step again when it does not return). Stays: the health look before it |
| `doctor` | `cmd/doctor.rs` | the key's place, the identity file, the record's CLI subcommands against `--help` (the CLI's name is now the record's `cli` key). Dropped: the old-size row, with the alias |
| `includes` | dropped | it flags an `Include` of a `.known` file under `~/.ssh/config.d`; nothing writes there any more |

## `crates/core/src/runners.rs` (359 lines)

| function | new home | notes |
|---|---|---|
| `Verb` | `cmd/runner.rs` | `add`, `rm`, `revive`, `tokens`; `Status` is the pool rows of `ls`. Dropped: `RetireLocal` |
| `Gh` | `runner.rs` `Registered` | a struct with named fields instead of a tuple |
| `conf` | `config.rs` | `[runners.<pool>]`; more than one pool is allowed |
| `log` | `log.rs` | |
| `vms` | `log.rs` `pool_vms` | `runner-add` less `runner-rm`. Dropped: the `vms` config key; the import writes a line per entry |
| `github` | `runner.rs` `registered` | `gh api` on the main host, by label |
| `deregister` | `runner.rs` | |
| `token` | `runner.rs` `registration_token` | returns a `Secret` |
| `rc` | `record.rs` `Record::verb`, `Record::inside` | the `runner_*` keys are the `boat-api` record's verbs and its `[runner]` lines |
| `per_day` | `spend.rs` | the record's rate × 24 |
| `approved` | dropped from the helper; stays | an owner ask is drover's. The helper's caps are hard, and `--last` is the one confirmation it takes; drover checks its ask before it passes `--last` or proposes a cap edit |
| `ask_owner` | stays | drover turns exit 3 `RUNNERS_CAP` and `RUNNERS_LAST` into its ask |
| `health` | `runner.rs` `health` → `ls --json` `online` | Stays: the `runners-low` and `runners-ok` alerts, read from the pool row |
| `push_tokens` | `runner.rs` | |
| `tick` | `tick.rs` → `runner.rs` `pass` | the refresh interval is the pool's `token_refresh_minutes` instead of drover's `[limits]` |
| `run`: `Status` | `cmd/ls.rs` | the pool row and its VMs' rows; each VM's provider state is read by the pass, not by `ls` |
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
| `hosts.rs` `at`: `cloud::held` | the row's `hold` and `state` |
| `hosts.rs` `wake`: `cloud::start` | `shed-cloud start <vm>` |
| `hosts.rs` survey: `cloud::moved` | deleted; the pass keeps the machine's address |
| `cli.rs`: `host destroy`, `hosts snapshot` | `shed-cloud rm <vm> --yes`, `shed-cloud snapshot <vm>` after drover's health look |
| `cli.rs`: `hosts bootstrap` | drover's own, minus its start |
| `cli.rs`: `drover runners <verb>` | `shed-cloud runner <verb>`, or deleted, as drover#1644 decides |
| `doctor.rs`: `cloud::doctor`, `cloud::includes` | `shed-cloud doctor`'s rows; `includes` gone |
| `serve.rs`: `cloud::card` | drover's card over the row |
