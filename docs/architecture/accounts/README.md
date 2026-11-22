# Accounts, login, the token proxy and usage in shed (#256)

## Summary

Decision: shed keeps the accounts on the main host: one config folder per account, an optional setup token beside it, and what it last read of each account's sign-in and usage. A run names an account or `auto`; shed sets the account's variables itself, picks the account with the most headroom on `auto`, and moves a lane to the next account when it hits a limit. A run on another machine never gets a token: it gets a key made for that one run, and shed's proxy on the main host signs its calls. Every provider fact is a key of the provider's record. This reverses three earlier decisions: shed's "never handles provider credentials", drover #923 and cooee #470.
Slices: in shed, after the hotspots slices #248 to #250: (1) record keys, readers and redaction; (2) the account store, login, status and remove; (3) a run on a named account; (4) usage, `auto` and the switch on a limit; (5) the token proxy; (6) the machine, file and process APIs drover's split needs; (7) tagged client releases. Then cooee moves its accounts in, then drover. About 2,050 lines added in shed, about 2,200 removed from cooee and drover.
Open questions: (1) shed switches between accounts of one agent; a chain that crosses agents (Claude, then Codex) stays a cooee card: accept (recommended)? (2) `auto` never picks the `default` login, which is the owner's own config folder: accept (recommended)? (3) the keys need SHA-256: add the `sha2` crate (recommended, drover uses it) or write it in shed? (4) one cap raise of about 2,100 lines in its own cap issue, sized by `drover cap shed` when filed. (5) `account login --token` reads the setup token off the screen and saves it, so nobody pastes one (recommended), or the hand-saved file only?

The question, the options, the decision, what it reverses, its consequences and the `arch.toml` lines:
[adr.md](adr.md). Types, methods, error codes and record keys: [shapes.md](shapes.md). The security model:
[security.md](security.md). This file has how the callers read, the statecharts, the proxy's path, the
boundaries, the migration order and the issues to file.

## How it reads

### cooee: the sign-in sheet

Today `Logins` (cooee-server `login/`) keeps the folders, builds `env VAR=<folder> <login command>`, redacts
each screen it reads, runs the status command every 30 s and parses `/usage`. After:

```rust
let accounts = shed.accounts(Some("claude"))?;                    // account.ls, in the order `auto` picks
for a in &accounts {
    sheet.row(&a.name, a.state, a.who.as_deref(), a.usage.as_ref(), a.limited_until);
}

shed.account_add(&AccountAdd { provider: "claude".into(), name: "work".into(), ..AccountAdd::default() })?;
let login = shed.account_login("claude", "work", false)?;         // the record's login, in a redacted terminal
for update in shed.watch(&login.terminal, None)? {                // the screen is redacted by shed
    sheet.screen(update);
}
shed.send(&SendParams { name: login.terminal.clone(), text: code, ..SendParams::default() })?;
shed.wait_state(&login.terminal, &[], 600.0)?;                  // returns when the login exits
let account = shed.account_status("claude", "work")?;             // signed in, or its line says why not
```

### cooee: a thread on an account

```rust
shed.run(&RunParams {
    acp: Some(agent), account: Some(thread.account.clone()),      // "work", "default" or "auto"
    meta, mcp_servers, cwd, labels, ..RunParams::default()        // no folder variable in `env` any more
})?;

match event.kind {
    EventKind::Account { account, was: Some(was), .. } => thread.note(format!("{was} hit its limit; now on {account}")),
    EventKind::State(s) if s.state == "rate-limited" => thread.card(cross_agent_options()),   // cooee's own ask
    _ => {}
}
```

### drover: a lane, on this host or another

```toml
[fleet.shed.role.impl]
harness = "claude"
account = "auto"          # or a name; unset keeps today's host login
```

```rust
let run = RunParams { acp: Some(id), account: role.account.clone(), machine: host.machine(), meta: guard, ..run };
match shed.run(&run) {
    Err(e) if e.code() == Some(ErrorCode::AccountSignedOut) => needs_human("LOGIN_NEEDED", login_card(&e)),
    Err(e) if e.code() == Some(ErrorCode::AccountLimited) => hold(issue, first_reset(&shed.accounts(None)?)),
    Err(e) if e.code() == Some(ErrorCode::NoProxy) => refused("NO_PROXY_TOKEN", e.to_string()),
    other => other?,
};

// the LOGIN_NEEDED card's [sign in]: the same three calls as cooee's sheet
// the Hosts screen's key list
for k in shed.proxy_keys()? { row(&k.id, k.state, &k.terminal, &k.account) }
shed.proxy_revoke(&id)?;
```

### drover: its hosts as shed machines

```rust
shed.machine_add("box", "user@box.example")?;                     // from [hosts.box] machine = "…"; twice is once
let link = &shed.links()?["box"];
let offline = link.down_since.is_some_and(|t| now - t > cfg.limits.host_offline_minutes * 60);
let tags = [link.os.clone(), link.arch.clone()];                  // xcode, docker, webkit stay drover's, by `exec`

let s = shed.stats(&StatsParams { machine: Some("box".into()), top: Some(3), since })?;
admit(s.now.mem_free_mb, s.now.load_per_cpu, swap_rate(&s.history), &s.top_cpu);
let tree = shed.procs("box/lane-256-impl")?;                      // was `ps` and `lsof` over ssh, joined by cwd

shed.file_write(&FileParams { path, data: Some(bytes), mode: Some(0o600), machine: host.machine(), ..FileParams::default() })?;
shed.exec(&ExecParams { cmd, env, machine: host.machine(), ..ExecParams::default() })?;   // was `ssh host -- …`
```

### the owner, in a terminal

```sh
shed account add claude work
shed account login claude work            # the login screen, redacted; type the code
shed account login claude work --token    # the setup token, saved 0600; needed for runs on other machines
shed account ls                           # NAME  STATE  WHO  5H  7D  TOKEN  RUNS
shed run --acp claude-acp --account auto
shed proxy keys
```

## Statecharts

### An account

```
            account.add               signed in, or a token saved
 (none) ───────────────▶ signed-out ─────────────────────────────▶ ready ◀──────────────────┐
    ▲                        ▲    ▲                                  │                       │
    │ account.rm             │    │      signed out and no token     │ a limit               │ the reset passes
    │ (any state, no run)    │    └──────────────────────────────────┤                       │
    └────────────────────────┘                                       ▼                       │
                                                                  limited ───────────────────┘
```

| edge | what shed saw |
|---|---|
| signed in | a login ended, or `account.status` ran, and the status command exits 0 |
| a token saved | `account.login` with `token`, or `account.add` with `token_file` |
| signed out | the status command says so, or a lane on the folder's login showed the record's auth-failure line; a token the proxy's upstream answers 401 to is marked `rejected` and counts as no token until a new one is saved (the file stays) |
| a limit | a lane on the account showed the record's limit line, or the stream driver's limit event. An upstream 429 marks nothing |
| the reset passes | `limited_until`: the reset the limit event carries, else the reset of the fullest window in a usage reading taken at once, else now plus the record's `usage.hold`. A later reading under 100 % ends it early |

### A login

```
 account.login ─▶ running (terminal `login-<provider>-<name>`, redacted, at most 600 s)
                     │ the terminal exits, or the 600 s pass (shed kills it)
                     ▼
                  checking: the status command; with `token`, the token file written from the screen
                     │
                     ├─ signed in ─▶ the account is `ready`; `checked_at` is now
                     └─ not ───────▶ the account stays as it was; its `line` says why
```

A second `account.login` for the same account while one runs returns the running terminal.

### A run's account

```
 run {account} ─▶ resolve ──▶ refused: NO_ACCOUNT, ACCOUNT_SIGNED_OUT, ACCOUNT_LIMITED,
                     │                 ACCOUNT_LOCAL_ONLY, ACCOUNT_SETUP, NO_PROXY
                     ▼
                on account A  (event `account`, why `start`)
                     │ the lane hits a limit: A becomes `limited`
                     ├─ a named account ──────────────────────────▶ rate-limited, as today
                     ▼ `auto`
                 pick again ── none left ─────────────────────────▶ rate-limited, note: the first reset
                     │ B
                     ▼
                 switching:  main host: stop the agent, copy the session's files A → B, start on B, `session/load`
                             a machine: the run's key now signs with B's token; the agent keeps running
                     ▼
                on account B  (event `account`, was A, why `limit`), then one prompt: go on
```

A `rate-limited` lane on `auto` resolves again when its next turn starts, so the caller's `send` after the
reset lands on whichever account is ready. shed starts no turn by itself, as today. A switch happens only on a
limit: moving a healthy lane would cost it its prompt cache.

## The proxy's path

```
 machine                                            main host
 ┌──────────────────────────────────────┐           ┌───────────────────────────────────────────────┐
 │ agent ─▶ 127.0.0.1:<relay port>      │           │ shed serve                                    │
 │   base URL and key in its env        │           │   proxy.sock (0600)                           │
 │ shed serve: relay, bytes only        │   ssh -R  │     key's sha256 in proxy-keys.json? live?    │
 │   ─▶ up-<nonce>.sock ════════════════╪═══════════╪═▶   path under the record's proxy.paths?      │
 └──────────────────────────────────────┘           │     the account's token ─▶ curl -K - ─▶ upstream
                                                    └───────────────────────────────────────────────┘
```

When a link comes up, the main host asks the machine `proxy.relay`. The shed there keeps one loopback port for
good (in its state file), names a fresh socket path, and answers both. The main host then holds one
`ssh -N -R <that path>:<proxy.sock>` beside the link and restarts it with a new path whenever it ends.
`Link::proxy` is the port while that `ssh` runs. A dropped link leaves the agents' base URL valid: their calls
fail until the forward is back, and the CLI retries. When the main server starts, a key whose terminal `ls`
over the link no longer shows is ended.

## Module boundaries

```
  shed (CLI) ──▶ shed-server ──▶ shed-accounts ──▶ shed-core   (runner, clock, redact)
      │              │   │              └────────▶ shed-proto  (Provider and its readers, Account, ProxyKey)
      │              │   └──▶ shed-catalog ──────▶ shed-proto
      ▼              ▼
  shed-client ───────────────────────────────────▶ shed-proto
      ▲
  cooee, drover: shed-client and shed-catalog at one tag; no account, login, usage or proxy code of their own
```

| crate | file | holds | may call |
|---|---|---|---|
| shed-proto | `accounts.rs` | the wire types of [shapes.md](shapes.md); `Provider::read_status`, `read_usage`, `when` | nothing |
| shed-core | `redact.rs` | `redact` | nothing |
| shed-accounts | `store.rs` | folders, links, `accounts.json`; add, adopt, remove; the `setup_files` check | core |
| | `token.rs` | `Token`; reading a 0600 file, saving one | core |
| | `check.rs` | the status command and the usage probe, through the runner | `store`, core, proto |
| | `choice.rs` | `Choice`, the ranking; pure | proto |
| | `env.rs` | `Injected`: what a run on the main host gets and loses | `store`, `token`, `keys` |
| | `keys.rs` | mint, find by sha256, end, revoke; `proxy-keys.json` | core |
| | `proxy.rs` | the Unix listener, the checks, the upstream call | `keys`, `token`, core, proto |
| | `relay.rs` | the machine side: loopback TCP to the forwarded socket | core |
| | `session.rs` | copying a session's `session_files` between two folders | core |
| shed-server | `methods/account_*.rs`, `proxy_*.rs`, `machine_*.rs`, `terminal_procs.rs` | one handler per method (#249's layout) | accounts, the registry |
| | `lane.rs` | on a limit: mark, pick, switch (the hibernation stop and wake, with another account) | accounts |
| | `links.rs` | `down_since`, the machine's facts, the forward's `ssh` and `Link::proxy` | accounts (`relay` over the link) |
| shed | `cmd/account.rs`, `cmd/proxy.rs`, `doctor/accounts.rs` | the commands and doctor rows (#250's layout) | client |

Rules, each with its enforcer:

- `shed-accounts` uses core and proto only; only the server uses it (`just arch`, the `accounts` layer).
- It starts every process through `shed_core::runner` and reads the clock through `shed_core::clock` (the
  existing bans).
- It names no provider: every provider fact comes in as a `Provider` (`just neutral`).
- `Token::expose` is called in `env.rs` and `proxy.rs` only; `TcpListener` is in `relay.rs` only (two new bans).
- `choice.rs` does no I/O: it takes `&[Account]` and returns a name, so its ranking is tested as a table.
- Apps read; they never write under `accounts/`. Nothing enforces that from shed's side but the folder's mode.

## Migration order

Nothing below needs a flag day: each step leaves all three programs working.

| step | repo | what | needs |
|---|---|---|---|
| 0 | shed | hotspots slices #248, #249, #250 | queued already |
| 1 | shed | slice 6 (machine, file, process APIs) and slice 7 (tags): no account in them, so they can go first | 0 |
| 2 | shed | slices 1, 2, 3: records, the store and login, a run on a named account | 0 |
| 3 | cooee | accounts move: `shed account add --folder` adopts each of cooee's folders in place (same path, so the same Keychain item: no new sign-in); the sheet, the picker and threads call shed; cooee's store, login runner, usage parser and `cooee accounts env` are deleted | 2 |
| 4 | shed | slice 4: usage, `auto`, the switch | 2 |
| 5 | cooee | a thread may be on `auto`; the fallback card keeps only its cross-agent options | 3, 4 |
| 6 | shed | slice 5: the token proxy | 2 |
| 7 | drover | client on a tag; `[hosts]` become shed machines; probes, `ps` and the secrets file go through shed | 1 |
| 8 | drover | accounts: the role key, LOGIN_NEEDED from shed, `shed account add --token-file` takes `[proxy] token-file` over, `proxy.rs` and the tunnel are deleted, `hosts keys` reads shed | 4, 6, 7 |

Between steps 6 and 8 two proxies exist, drover's and shed's, each with its own token file and port; they do
not meet. Step 8 deletes drover's in the same PR that switches its lanes, and its docs say #923 is reversed.

## The issues to file

In shed, each blocked on #248 to #250:

| # | title | scope | src lines |
|---|---|---|---|
| S1 | accounts 1: record keys, the status and usage readers, `redact` in core | the keys and tables of [shapes.md](shapes.md) in `Provider` and the three records, from cooee's and drover's facts; `read_status`, `read_usage`, `when` with cooee's test cases; `redact` with cooee's tests; `TerminalInfo::redacted` and every read path through it; `docs/accounts/login.md` | +300 |
| S2 | accounts 2: the store and the lifecycle | crate `shed-accounts` (`store`, `token`, `check`); `account.add`, `ls`, `login`, `status`, `rm`; `shed account`; doctor rows (a token file's mode and age, a signed-out account, setup files); the `accounts` layer and the `.expose()` ban | +500 |
| S3 | accounts 3: a run on a named account | `RunParams::account`, `env.rs`, the scrub, the refused `env` keys, `TerminalInfo::account`, the `account` event, a wake and a server restart that set the account again; `docs/acp/accounts.md` deleted for `docs/accounts/accounts.md`, which states the reversal; `PROTOCOL` 5 | +200 |
| S4 | accounts 4: usage, `auto` and the switch on a limit | first a live test that a session copied between two folders loads; the probe's beat, `choice.rs`, limit marks, `session.rs`, the switch in `lane.rs`, the go-on prompt; `docs/acp/lane-states.md` loses "nor switches its account" | +350 |
| S5 | accounts 5: the token proxy | `keys`, `proxy`, `relay`; `proxy.keys`, `revoke`, `relay`; the forward in `links.rs`; a run on a machine; the key re-pointed on a switch; `account login --token`; the `TcpListener` ban; drover's proxy tests moved | +450 |
| S6 | shed APIs for drover's split | `file.write` `mode`; `exec` `env`; `Link::down_since`, `hello`, `os`, `arch`, `home`; `Sample`'s new fields, `top`, `machine`; `terminal.procs`; `machine.add`, `machine.rm` | +250 |
| S7 | release: a tag per client change | the release workflow tags main `client-v<PROTOCOL>.<n>` when `shed-proto`, `shed-client` or `shed-catalog` changed since the last tag; `docs/releases.md` | 0 |
| cap | cap: raise for the accounts slices | one raise, sized by `drover cap shed` on the day | 0 |

In cooee:

| # | title | scope |
|---|---|---|
| C1 | accounts are shed's | adopt the folders; the sheet, picker and `Logins::home` on `account.*`; a profile's settings go in `meta`, never in the folder variable; delete `accounts.rs`, the login runner and checks, `read_status`, `read_usage`, `cooee accounts`; `docs/accounts.md` and `docs/logins.md` say #470 is reversed |
| C2 | a thread on `auto` | the thread's account may be `auto`; the `account` event as a thread line; `fallback.rs` keeps the cross-agent card for a `rate-limited` lane and drops the same-agent seats |

In drover:

| # | title | scope |
|---|---|---|
| D1 | hosts are shed machines | `[hosts.<n>] machine` to `machine.add`; offline from `down_since`; admission from `machine.stats`; `top` from `terminal.procs`; `write_private` to `file.write`; `On`'s ssh runner to `exec`; client pinned to a tag |
| D2 | accounts are shed's | `account` on a fleet and a role; LOGIN_NEEDED and the limit hold read `account.ls`; the card's sign-in calls `account.login`; `ACP_UNGUARDED` refuses an account with a setup; `proxy.rs`, the tunnel, `[proxy]` and `NO_PROXY_TOKEN`'s own check deleted; `hosts keys` on `proxy.keys`; `docs/accounts.md` says #923 is reversed |
