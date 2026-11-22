# Hosts, VMs, installs, secrets and process limits in shed (#259)

## Summary

Decision: shed does the host-side work and takes no decision about it. It installs a checked release of any tool on any machine, makes and runs VMs from one data file per VM provider (cloud hosts, runner VMs and Apple container desks alike), keeps a line of slots for heavy commands on each machine, runs a terminal or a command inside a sandbox when asked, and reads an API-key provider's balance. Secrets get one implementation, a shed crate that keeps them in the Keychain on macOS and in 0600 files on Linux; each program reads only its own, in its own process, and no shed method ever returns one. drover and cooee keep every decision: which build and when, budgets and idle stops, runner registration, which commands are heavy, who gets a secret, a desk or a sandbox.
Slices: in shed, after #256's slices: (1) the secrets crate and shed's own secrets; (2) installs; (3) slots; (4) VMs, shed on a new machine and machine setup; (5) the sandbox option and the inside socket; (6) balance probes. Each is followed by one cooee or drover issue that moves its callers and deletes the old code. About 2,250 lines added in shed, about 2,400 removed from drover and cooee.
Open questions: (1) apps read their own secrets through the crate and shed returns none (recommended), or shed's server holds every secret and returns values to cooee and drover over its socket? (2) the Keychain needs the `security-framework` crate, which cooee already uses: add it (recommended)? (3) a sandboxed run reaches a second shed socket that only takes state reports, events and slots, so a sandboxed agent cannot start terminals: accept (recommended)? (4) one cap raise of about 2,300 lines in its own cap issue, sized by `drover cap shed` when filed.

The question, the options, the decision, what it reverses, its consequences and the `arch.toml` lines:
[adr.md](adr.md). Types, methods, error codes and record keys: [shapes.md](shapes.md). The security model:
[security.md](security.md). This file has the split, how the callers read, the statecharts, the boundaries, the
migration order and the issues to file.

## The split

| area | shed: the mechanism | drover and cooee: the policy that stays |
|---|---|---|
| installs | find a release; stage it (download, sha256, version, signature, gate); swap; put the old one back; shed itself on a new machine | which branch and build; pause; how long to wait for a release or a lagging host; the swap order (main host last) and the `HOST_UPDATING` hold; what runs after a swap (profiles sync, serve handover, service restart, the port check); whether to roll back; the failed mark; every alert |
| cloud VMs | the provider records; create, start (wait, address, pinned host key), stop, destroy, snapshot, state; the VM as a shed machine; the setup step runner | the monthly budget and hours a day; spend (the record's rate × the time drover logged); start on demand and overflow; idle stop; the wip save before a stop; size; destroy is the owner's; snapshot only when healthy; the health checks (git, gh, the token's expiry); the setup step list |
| runner VMs | create, state, delete, and a command run inside the VM | registration tokens and their 30-minute refresh; listing and deregistering runners; `max_vms` and `usd_day`; owner approvals; the drain rule; `retire-local`; `runners-low`; the VM-side command lines |
| secrets | the store crate; shed's own secrets (a VM provider's key); the memory-only fields a value travels in | which fleet, persona or MCP server gets which secret; the `secret:<NAME>` syntax; the request card and who may save; typing one only into a program's terminal; the wip secret scan; events |
| slots, processes | the line, its limits, leases held by a connection; the process tree (#256's `terminal.procs`) | which commands are heavy (the kind patterns) and the hook that rewrites them; the limit numbers per host; the wait, and whether a late command runs anyway; `hold`'s admission; `top`'s layout |
| sandbox | the settings file, link resolution, the wrap, the agent's own needs from its record, shed's own denies, the inside socket | whether a fleet is sandboxed; the folders and hosts its toolchains and repos need |
| balance | the record's keys, the probe, the reading on the account | `warn_below`, the alert, the board's row |
| container VMs | the `container` record, the host-only network, the reach probe, lifecycle, a command inside | which persona has a desk; image and caps and their limits; the blocked addresses; the desk MCP tool (screenshot, click, type); the owner's lease; the replay log |

## How it reads

### drover: one self-update pass

Today `update.rs` and `release.rs` build shell lines (`curl`, `sha256sum`, `mv`) and run them over ssh. After:

```rust
let src = Source { tool: "drover".into(), releases: releases_url(&f.repo), tag: tag(branch, head) };
if let Err(e) = shed.install_find(&src) {
    return wait_or_alert(head, &e);                                 // NO_RELEASE: drover's 45 minutes, then its alert
}
for m in members.iter_mut().filter(|m| m.needs(head) && !m.staged(head)) {
    let gate = if m.has_config { vec!["doctor".into(), "--config-only".into()] } else { vec![] };
    let stage = Stage { machine: m.machine(), source: src.clone(), build: head.into(), dest: m.exe.clone(),
                        gate, identifier: Some("drover".into()) };
    match shed.install_stage(&stage) { Ok(i) => m.builds = i, Err(e) => m.failed = Some(e) }
}
match step(now, head, &members, &marks, &cfg.limits) {              // unchanged: current, wait, hold, stuck, swap
    Step::Swap { ready, .. } => {
        for m in remote(&ready) {
            shed.install_swap(&Swap { machine: m.machine(), dest: m.exe.clone(), build: head.into(), links: m.links() })?;
            shed.exec(&profiles_sync(m))?;
        }
        shed.install_swap(&Swap { dest: live.clone(), build: head.into(), ..Swap::default() })?;   // the main host last
        hand_serve_over()?;
    }
    other => report(other),
}
```

### cooee: its update

```rust
let head = decide(REV, failed.as_deref(), &main_head()?)?;          // cooee's: auto_update, the failed mark
shed.install_stage(&Stage { source: source(&head), build: head.clone(), dest: bin.clone(), ..Stage::default() })?;
shed.install_swap(&Swap { dest: bin.clone(), build: head.clone(), ..Swap::default() })?;
if !restart(config) {                                               // cooee's: the port must answer
    shed.install_back(&bin)?;
    restart(config);
    push_note(&head);
}
```

### drover: a cloud host and a runner VM

```rust
// placement chose boat1, and its budget allows it (drover's)
let vm = shed.vm_start(&VmStart { name: "boat1".into(), ..VmStart::default() })?;   // waits; address read, key pinned, link up
log("cloud-up", &vm);
shed.machine_setup("boat1", &steps(cfg))?;                          // drover's list: git, gh, rust, clones; shed runs each

// check, every 5 minutes
for vm in shed.vms(true)? {                                         // asks each provider now
    if we_thought_running(&vm) && vm.state != VmState::Running { log_stop(&vm, "stopped outside drover"); continue; }
    if over_budget(spent(&events, vm.rate_usd_hour)) || idle(&vm) {
        save_wip(lanes_on(&vm))?;
        shed.vm_stop(&vm.name)?;
    }
}

// runners add: the cap and the owner's approval are checked first (drover's)
let vm = shed.vm_create(&VmCreate { provider: "boat-api".into(), size: c.size.clone(), vars: runner_vars(c), ..VmCreate::default() })?;
let token = registration_token(c)?;                                 // drover's: gh
shed.vm_exec(&VmExec { name: vm.name, cmd: c.revive.clone(), stdin: token.into_bytes(), ..VmExec::default() })?;
```

### cooee: a desk

```rust
shed.vm_create(&VmCreate { provider: "container".into(), name: Some(desk.into()), image: Some(image), vars: caps.vars(),
                           labels: persona_label(&p), ..VmCreate::default() })?;
shed.vm_start(&VmStart { name: desk.into(), probe: cfg.blocked.clone(), ..VmStart::default() })?;   // VM_REACHED stops it

// the desk MCP tool stays cooee's; its three actions are commands inside the VM
let png = shed.vm_exec(&VmExec { name: desk.into(), cmd: "env DISPLAY=:1 import -window root png:-".into(), ..d })?.stdout;
shed.vm_exec(&VmExec { name: desk.into(), cmd: format!("env DISPLAY=:1 xdotool mousemove {x} {y} click 1"), ..d })?;
shed.vm_exec(&VmExec { name: desk.into(), cmd: "env DISPLAY=:1 xdotool type --file -".into(), stdin: text.into_bytes(), ..d })?;
```

### drover: a lane with secrets, in a sandbox

Today spawn writes `<worktree>.env` (0600) on the lane's host, the pane's shell sources it, spawn deletes it, and
`<worktree>.srt.json` wraps the harness command. After:

```rust
let store = shed_secrets::Store::open(drover_place(ctx));           // drover's own process, the main host
let mut env = plan.env.clone();
for (var, name) in plan.secrets() {
    env.insert(var, store.get(&name)?.ok_or_else(|| missing(&name))?.expose().to_owned());
}
shed.run(&RunParams {
    acp: Some(id), account: role.account.clone(), machine: host.machine(), meta: guard,
    env,                                                             // memory only, on both ends (#191)
    sandbox: f.sandbox.then(|| Sandbox { write: vec![worktree, git_dir, target, scratch], read: toolchains(),
                                         hosts: registries(), ..Sandbox::default() }),
    ..run
})?;
```

### drover: heavy commands and `top`

```rust
// the lane's hook (drover's): a command matching a heavy kind is rewritten to
//   shed slot run build --wait 30m -- cargo test
shed.slot_limits(host.machine(), h.heavy_max, &h.heavy_kind_max)?;  // at spawn; twice is once

let slots = shed.slots(host.machine())?;
for lane in &lanes {
    let tree = shed.procs(&lane.terminal)?;                         // #256
    let waits = slots.line.iter().filter(|t| !t.running && t.holder == lane.key);
    render(lane, tree.iter().map(|p| (p, heavy.kind(&p.command))), waits);
}
```

### drover: a balance

```rust
for a in shed.accounts(Some("deepseek"))? {
    if let Some(b) = &a.balance { row(&a.name, b); if b.total < p.warn_below { alert(&a.name, b) } }
}
```

### the owner, in a terminal

```sh
shed secret set BOAT_API_KEY              # the value from stdin, never an argument
shed secret ls                            # shed's own: NAME SAVED; no values
shed machine add box user@box --install   # shed's release there, checked, then its jobs
shed vm ls                                # NAME PROVIDER STATE SINCE MACHINE
shed install ls ~/.local/bin/cooee --on box
shed slot ls --on box
shed slot run build --wait 30m -- cargo test
```

## Statecharts

### An install at one path on one machine

```
 live L ──install.stage──▶ live L, staged N ──install.swap──▶ live N, prev L ──install.back──▶ live L
              │ download, checksum, version, signature or gate fails
              ▼
           live L: the staged file is removed, UNSTAGED names the check
```

The files are the state (`<dest>`, `<dest>.new`, `<dest>.prev`); shed keeps no record of its own. Staging the
build that is already staged, or swapping to the build that is already live, changes nothing.

### A VM

```
          vm.create              vm.start                 the provider says running; address read; key equals the pin
 gone ─────────────▶ stopped ─────────────▶ starting ──────────────────────────────────────────────▶ running
   ▲                   ▲  ▲                    │ 5 minutes pass, the key changed, or a probed address answers │
   │ vm.destroy        │  └────────────────────┘ shed stops it: VM_START, VM_KEY_CHANGED, VM_REACHED          │
   └───────────────────┴──────────────────── vm.stop, or the provider says so (event `vm`, why `outside`) ────┘
```

shed never starts, stops or destroys a VM by itself, except to stop one whose start failed. A VM whose record has
`address` is also a shed machine of the same name: its link is `down` while the VM is stopped.

### A slot ticket

```
 slot.take ─▶ waiting ──── first in line with a slot and its kind free ───▶ running ── connection closes ─▶ gone
                 │ `wait_s` pass                                                       or slot.free
                 ▼
               late: answered `admitted: false` and out of the line; the caller runs it anyway or gives up
```

## Module boundaries

```
  shed (CLI) ──▶ shed-server ──▶ shed-hosts ─────▶ shed-core     (runner, clock)
      │              │   │           ├───────────▶ shed-proto    (Install, Vm, VmProvider, Ticket, Sandbox)
      │              │   │           └───────────▶ shed-secrets  (Store, Secret)
      │              │   ├──▶ shed-accounts (#256): the balance probe
      │              │   └──▶ shed-catalog ──────▶ shed-proto    (providers/*.toml, vms/*.toml)
      ▼              ▼
  shed-client ───────────────────────────────────▶ shed-proto
      ▲
  cooee, drover: shed-client, shed-catalog and shed-secrets at one tag; no install, VM, slot, sandbox or keychain code
```

| crate | file | holds | may call |
|---|---|---|---|
| shed-secrets | `lib.rs`, `keychain.rs`, `files.rs` | `Store`, `Secret`, `Place`; the Keychain (macOS), 0600 files; the 5 s limit | nothing |
| shed-proto | `install.rs`, `vm.rs`, `slot.rs`, `sandbox.rs`, `secret.rs` | the wire types of [shapes.md](shapes.md), one area file each; `Provider::read_balance` | nothing |
| shed-hosts | `install.rs` | find, stage, swap, back, ls and their checks; shed's own self-update calls it through the server, as it calls the runner | core, proto |
| | `first.rs` | shed on a new machine, over ssh | `install`, core |
| | `setup.rs` | the step runner: check, install, read back | core, proto |
| | `vm.rs` | a record's verbs, filled and run; `vms.json` | core, proto, secrets |
| | `pin.rs` | a VM's address, its pinned host key, its ssh options | core |
| | `slots.rs` | the line: tickets in, the admitted out; pure | proto |
| | `sandbox.rs` | the settings file, link resolution, the wrap | core, proto |
| shed-accounts | `check.rs` | the balance probe beside #256's usage probe | as in #256 |
| shed-server | `methods/install_*.rs`, `vm_*.rs`, `slot_*.rs`, `secret_*.rs`, `machine_setup.rs` | one handler per method (#249's layout) | hosts, secrets, the registry |
| | `inside.rs` | the inside socket and the methods it serves | the registry |
| | `links.rs` | a VM machine's ssh options at connect | hosts (`pin`) |
| shed | `cmd/install.rs`, `cmd/vm.rs`, `cmd/slot.rs`, `cmd/secret.rs`, `doctor/hosts.rs` | the commands and doctor rows (#250's layout) | client |

Rules, each with its enforcer:

- `shed-secrets` uses no workspace crate; `shed-hosts` uses core, proto and secrets, and only the server uses it
  (`just arch`, two new layers).
- `shed-hosts` starts every process through `shed_core::runner`, a record's verb included (the existing ban).
- It names no VM provider: every provider fact is a record (`just neutral`, whose word list gains `boat` and
  `deepseek`).
- `Secret::expose` is called in `vm.rs` and the store's two files only (#256's `.expose()` ban, three more files).
- `slots.rs` does no I/O: its admission order is tested as a table, with drover's cases.
- The inside socket serves its six methods and no other: a test calls every method on it.

## Migration order

After #256's steps 0 to 8, in the same table. Each step leaves all three programs working, and each app step
deletes the code it replaces in the same PR.

| step | repo | what | needs |
|---|---|---|---|
| 9 | shed | H1: the secrets crate, `secret.*` for shed's own, the tag carries the crate | #256 step 1 (tags) |
| 10 | cooee | C3: secrets on the crate, at the Keychain service and file folder it has today (nothing moves); `secrets.rs` deleted | 9 |
| 11 | drover | D3: secrets on the crate; on macOS its files move into the Keychain once; a lane's secrets go in `run`'s `env`; the `.env` file, `Delivered` and `secrets-left` deleted | 9, #256 step 7 |
| 12 | shed | H2: installs; shed's self-update calls the same code | #256 step 1 |
| 13 | drover | D4: self-update on `install.*`; `release.rs` and the shell lines deleted | 12, #256 step 7 |
| 14 | cooee | C4: update on `install.*`; its fetch and swap deleted | 12 |
| 15 | shed | H3: slots | #256 step 1 |
| 16 | drover | D5: the hook rewrites to `shed slot run`; `hold` and `top` read shed; `heavy.rs` and the `ps` half of `top.rs` deleted | 15, #256 step 7 |
| 17 | shed | H4: VM records and lifecycle, VM machines, `machine add --install`, `machine.setup` | 9, 12 |
| 18 | drover | D6: cloud hosts and runner VMs on `vm.*`; recipes, the ssh config block, the pin file and bootstrap's installs deleted | 17, #256 step 7 |
| 19 | cooee | C5: desks on `vm.*`; the `cooee-desk` crate deleted | 17 |
| 20 | shed | H5: the sandbox option and the inside socket | 15, #256 step 2 (a run on an account) |
| 21 | drover | D7: `sandbox = "srt"` becomes `run`'s option, ACP lanes included; `sandbox.rs` deleted | 20, #256 step 7 |
| 22 | shed | H6: balance keys, the probe, an account that is only a key | #256 step 4 |
| 23 | drover | D8: the DeepSeek probe deleted; the board reads `account.ls` | 22, #256 step 8 |

Steps 9, 12, 15 and 22 do not depend on each other and can run side by side; 17 and 20 follow their two.
Between a shed step and its app step the old code still runs: nothing in shed takes over by itself. Step 18
is the one with live state to carry: drover registers each running cloud host with `vm.create` (it adopts a
VM the provider already has under that name) and hands its pinned key over; no VM restarts.

## The issues to file

In shed, each after #256's S6 and S7 and on #248 to #250's layout:

| # | title | scope | src lines |
|---|---|---|---|
| H1 | hosts 1: the secrets crate | crate `shed-secrets` from cooee's `secrets.rs` (Keychain, time limit, signed check, files, shred) and drover's 0600 refusal, with both test sets; `Secret`; `secret.set`, `ls`, `rm` and `shed secret` for shed's own; doctor rows (a wide file, a locked Keychain, an unsigned shed); the tag workflow carries the crate; `docs/hosts/secrets.md` | +400 |
| H2 | hosts 2: installs | `shed-hosts/install.rs` from shed's `update.rs`, drover's `release.rs` and cooee's `update.rs`; `install.find`, `ls`, `stage`, `swap`, `back`; `shed install ls`; self-update on it; `docs/hosts/installs.md` | +350 |
| H3 | hosts 3: slots | `slots.rs` with drover's admission cases; `slot.take`, `free`, `ls`, `limits`; `exec`'s `slot`; `shed slot run` and `ls`; a closed connection frees its slot; `docs/hosts/slots.md` | +300 |
| H4 | hosts 4: VMs and machine setup | `vms/boat.toml`, `boat-api.toml`, `container.toml` from drover's recipe and cooee-desk; `vm.rs`, `pin.rs`; `vm.*`; the `vm` event; VM machines in `links.rs`; `first.rs` and `machine.add`'s `install`; `setup.rs` and `machine.setup`; doctor rows from drover's recipe checks; `docs/hosts/vms.md`, `machines.md` | +750 |
| H5 | hosts 5: the sandbox option | `sandbox.rs` from drover's `sandbox.rs`; `run`'s and `exec`'s `sandbox`; the record's `[sandbox]`; `inside.rs`; drover's negative probes as tests; first a live run of an ACP lane under `srt` on macOS and on Linux; `docs/hosts/sandbox.md` | +330 |
| H6 | hosts 6: balance | the record's `[balance]`, `deepseek.toml`, `read_balance`, the probe, `Account::balance`, an account that is only a key; `docs/accounts/usage.md` gains a section | +120 |
| cap | cap: raise for the hosts slices | one raise, sized by `drover cap shed` on the day | 0 |

In cooee:

| # | title | scope |
|---|---|---|
| C3 | secrets are shed's crate | every `secrets::` call on `shed_secrets::Store` at today's place; `persona` naming stays; `secrets.rs` and the `security-framework` dependency deleted |
| C4 | update through shed | `fetch` and the swap become `install.stage`, `swap`, `back`; `decide`, the restart, the port check and the note stay |
| C5 | desks are shed VMs | create, wake, sleep, destroy and `made` on `vm.*`; the desk tool and the live view on `vm.exec`; caps and the blocked list stay cooee's; `cooee-desk` deleted; decision 0003 gets a note that shed holds the provider |

In drover:

| # | title | scope |
|---|---|---|
| D3 | secrets are shed's crate | `secret::file` and `path` on the crate; the macOS move; `run`'s `env` for a lane's secrets and its key; the card, `ls`, `rm` and events stay; docs say #707's files are the Keychain on macOS again |
| D4 | self-update through shed | `stage`, `swap_line`, `builds`, `signed`, `install_lines` deleted; `step`, the marks, the alerts, profiles sync and the handover stay |
| D5 | heavy slots are shed's | the hook's rewrite; `drover heavy` deleted, `hold` keeps admission and takes a shed slot; `top` joins `terminal.procs` and `slot.ls` |
| D6 | cloud hosts and runner VMs are shed VMs | `cloud.rs` keeps spend, budgets, idle stop, the wip save and the card; recipes become shed records (an override file moves to shed's config folder); `runners.rs` keeps registration and its caps; bootstrap keeps its step list and logins; `~/.ssh/config.d/herdsman` and the `Include` are no longer needed |
| D7 | the sandbox is a run option | `settings` becomes the three lists; `wrap`, `file`, `resolved` deleted; `ACP` lanes are covered |
| D8 | balances are shed's | `usage::deepseek` and `[providers.deepseek]`'s key file deleted; `warn_below` stays |
