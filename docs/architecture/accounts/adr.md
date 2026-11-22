# ADR: accounts, login, the token proxy and usage are shed's (#256)

## Summary

Decision: shed keeps the accounts on the main host: one config folder per account, an optional setup token beside it, and what it last read of each account's sign-in and usage. A run names an account or `auto`; shed sets the account's variables itself, picks the account with the most headroom on `auto`, and moves a lane to the next account when it hits a limit. A run on another machine never gets a token: it gets a key made for that one run, and shed's proxy on the main host signs its calls. Every provider fact is a key of the provider's record. This reverses three earlier decisions: shed's "never handles provider credentials", drover #923 and cooee #470.
Slices: in shed, after the hotspots slices #248 to #250: (1) record keys, readers and redaction; (2) the account store, login, status and remove; (3) a run on a named account; (4) usage, `auto` and the switch on a limit; (5) the token proxy; (6) the machine, file and process APIs drover's split needs; (7) tagged client releases. Then cooee moves its accounts in, then drover. About 2,050 lines added in shed, about 2,200 removed from cooee and drover.
Open questions: (1) shed switches between accounts of one agent; a chain that crosses agents (Claude, then Codex) stays a cooee card: accept (recommended)? (2) `auto` never picks the `default` login, which is the owner's own config folder: accept (recommended)? (3) the keys need SHA-256: add the `sha2` crate (recommended, drover uses it) or write it in shed? (4) one cap raise of about 2,100 lines in its own cap issue, sized by `drover cap shed` when filed. (5) `account login --token` reads the setup token off the screen and saves it, so nobody pastes one (recommended), or the hand-saved file only?

**Status:** accepted by the owner (#256); the five open questions take the recommendations above (the issue's
amendment of 2026-11-21: `sha2` for the key hashes, `--token` reads the token off the redacted screen).
Proposed by the architect lane, 2026-11-22, read on shed `a1f3716`, drover staging `0dcf23a0f` and cooee
staging `08e6fd2ad`. How the callers read, the API, the statecharts, the boundaries, the migration and the
issues: [README.md](README.md). Types and record keys: [shapes.md](shapes.md). The security model:
[security.md](security.md).

## The question

Three repos each own a part of "which login does this agent run on", and each wrote it per provider:

- **cooee** has the accounts: folders under its data folder (`accounts.rs`, 319 lines), the login in a shed
  terminal with a redacted screen (`login.rs` and `login/`, about 1,200 lines), the usage probe and its parser
  (`claude -p /usage`, read in `providers/claude.rs`), and the fallback card (`hub/fallback.rs`, 212 lines). A
  fleet engine was to get an account through `cooee accounts env` (#470).
- **drover** has no accounts (#923): a lane runs on the host's own login, and a usage limit or a sign-out holds
  the whole host until the owner signs in and turns it back on. For other hosts it has a token proxy
  (`proxy.rs`, 492 lines): `hsk-` keys kept as sha256, `[proxy] token-file`, the OAuth beta header, an `ssh -R`
  tunnel per host on a fixed port, and a script that kills whatever holds that port on the far side.
- **shed** runs every one of those agents and by contract knows nothing: "shed passes its environment through
  to the agent and never handles provider credentials" (`docs/acp/accounts.md`). It sees the limit line and the
  expired-login line of every lane, and can act on neither.

So the one program that starts every agent and sees every limit cannot choose a login, and the two that can
choose do not see the lanes of the other. What does shed own, what stays in the apps, and how does a run on
another machine sign in without holding a token?

## Options

**Who owns the accounts**

| | option | verdict |
|---|---|---|
| A | Leave it: cooee's accounts, drover's host login | two programs, two answers to "is this account capped"; a drover lane cannot use a second account |
| B | cooee serves accounts to drover (`cooee accounts env`, #470) | drover would need cooee running to start a lane; shed still cannot switch a lane it runs |
| C | **shed owns them: folders, login, status, usage, choice** | chosen (the owner's decision 1, 2, 4, 5) |

**What an account is**

| | option | verdict |
|---|---|---|
| D | shed stores the OAuth credentials and refreshes them | shed would follow every provider's token format and refresh rules |
| E | **A config folder the CLI keeps its own login in, plus an optional setup token file** | chosen: the CLI signs in and refreshes; shed sets one variable |
| F | Token files only | a token is good for a year and cannot read usage; the folder login is the one a person renews by signing in |

**How a run on another machine signs in**

| | option | verdict |
|---|---|---|
| G | Copy the account folder or the token to the machine | the credential is then on every host that ever ran a lane |
| H | Sign in on each machine | what #923 does: one login per host, by hand, and a limit holds the host |
| I | **The main host's proxy signs the calls; the run holds a key made for it alone** | chosen (decision 3) |

**How the machine reaches the proxy**

| | option | verdict |
|---|---|---|
| J | drover's way: `ssh -R` of one fixed TCP port on both sides | after a dropped link the far port is often still held; drover kills its holder, with `sudo` when needed |
| K | The proxy listens on the network | a token-signing port open to the network |
| L | **The shed on the machine owns a loopback port and relays it to a socket that `ssh` forwards to the main host's proxy socket** | chosen: the port agents were given outlives a dropped link, the forward gets a fresh socket path each time, and the main host opens no TCP port |

**Where the token goes on the main host**

| | option | verdict |
|---|---|---|
| M | Through the proxy too | the CLI then sees an API login (the record's `api_vars`), on the one host that already holds the file |
| N | **Into the run's environment, in the server's memory** | chosen: the owner's "injects for headless runs"; `run`'s `env` already has this contract |

**Choosing and switching**

| | option | verdict |
|---|---|---|
| O | The caller picks from `account.ls` and restarts its lane on a limit | cooee and drover would each write the ranking and the restart |
| P | **shed picks on `auto` and switches the lane itself** | chosen (decision 4): it already stops and reloads a session for hibernation |
| Q | Switch by starting a new session with a summary of the old one | what cooee's card does across agents; inside one agent the session itself can move |

## The decision

1. **shed holds the accounts, on the main host only.** An account is a provider, a name, a config folder shed
   made (or adopted by link), and optionally a setup token file at 0600. `default` is the CLI's own default
   folder. The shapes are in [shapes.md](shapes.md).
2. **shed runs the lifecycle**: `account.add`, `account.login`, `account.status`, `account.rm`, `account.ls`.
   Login is the record's own login command in a shed terminal marked `redacted`: every way to read that
   terminal passes through `redact`, which moves here from cooee. shed does no OAuth and no refresh.
3. **A run names its account.** `run`'s new `account` is a name or `auto`. On the main host shed sets the
   folder variable, and the token variable only when the folder's own login is signed out (a headless host
   whose Keychain is locked). It removes the record's `scrub` and `api_vars` variables it inherited, and
   refuses a caller `env` that sets any of them. Without `account`, shed sets nothing, as today.
4. **`auto` picks by headroom and switches on a limit.** Candidates are the provider's signed-in named accounts
   that are not limited; the one with the most headroom wins, then the one with fewer runs, then by name. When
   a lane on `auto` hits a limit, shed marks that account limited, picks again and moves the lane: on the main
   host it stops the agent, copies the session's files to the new folder and loads the session there; on
   another machine it points the run's key at the new account's token. Then it sends one fixed prompt to go
   on. With no candidate left the lane is `rate-limited`, as today, with the first reset in its note.
5. **shed reads usage.** It runs the record's usage command per account and reads the windows by the record's
   markers. A reading stands for `usage.fresh` seconds; accounts with a run are read on that beat, others when
   asked. Apps only call `account.ls`.
6. **The proxy is shed's, with a key per run.** `run` on a machine with an account mints a key, sends the far
   shed the relay's URL and the key as the run's environment, and ends the key when the terminal exits. The
   proxy checks the key's sha256, the request path against the record's `proxy.paths`, and signs the call with
   the account's token through `curl`, whose config goes over stdin. Upstream, auth header, added headers and
   variable names are the record's `[proxy]` table.
7. **Provider facts are record keys**: login, status and how to read it, the usage command and its markers,
   the limit and reset markers, the token command, the config folder variable, the proxy table, the files of a
   setup and of a session. A provider is one `.toml` file as long as its usage times are in a form `when`
   reads. cooee and drover keep policy: drover its guard `meta` and hooks, cooee its asks.
8. **The APIs drover's split needs** ship beside it: `file.write`'s `mode`, `Link::down_since` and the
   machine's facts, the new `machine.stats` fields with `top` and `machine`, `terminal.procs`, `machine.add`
   and `machine.rm`, `exec`'s `env`, and a tag on every client change.

What this reverses, said plainly:

- **`docs/acp/accounts.md`: "shed … never handles provider credentials."** Reversed. shed now holds account
  folders and setup token files, sets a run's login, and its proxy signs calls with a token. What stays true:
  shed stores no OAuth credential of its own, refreshes nothing, and never reads inside an account folder
  except the record's `setup_files` and `session_files`, and its `credentials.file`, read only to hash it for
  the drift check against the keychain item. The file is rewritten in slice 3.
- **drover #923: "drover has no account layer. Lanes run on the host's own login."** Reversed. A role or a
  fleet sets `account = "<name>"` or `"auto"`, and a limit or a sign-out is an account's state, read from shed,
  not a hold on the host. The role key `account` comes back; `accounts`, `fallback` and `[accounts]` stay
  retired. drover's `proxy.rs` is deleted.
- **cooee #470: "cooee owns its accounts"**, with `cooee accounts env` as the helper for a fleet engine.
  Reversed. shed adopts cooee's folders in place, cooee's sheet and picker call shed, and the helper, the
  account store, the login runner, the usage parser and the same-agent part of the fallback chain are deleted.

## Consequences

- One answer to "is this account capped", for every lane of both apps, and a capped account stops no host.
- A new provider with a usual CLI is a record file: no code in three repos.
- The main host is the one place with credentials. A machine that runs lanes can be given up without
  rotating anything: its keys die with their terminals.
- shed gains about 2,050 lines and a new crate, `shed-accounts`; the line cap needs one raise (question 4).
- shed's server becomes security-relevant in a new way: it reads token files and signs upstream calls.
  [security.md](security.md) lists what holds that, and each rule there names its test or its ban.
- Moving a session between accounts depends on the agent keeping a session in files under its config folder
  (the record's `session_files`). Slice 4's first test is a live run that proves it for Claude Code. An agent
  without `session_files` never switches on the main host: its lane stays `rate-limited`.
- Codex and Gemini have no `[proxy]` table, so an account of theirs runs on the main host only. On another
  machine they run as today: `account` unset, that machine's own login.
- A terminal that is not an ACP or stream lane gets its account at start and is never switched.
- `auto` draws on every signed-in named account of the provider. A fleet or a thread that must stay on one
  account names it.
- The `default` account is still the owner's own folder, so a lane on it is guarded by the caller's `meta`
  alone, exactly as now. A named account's folder cannot hold a setup at all.
- drover's rule "no rest timers" goes: an account has `limited_until`, and drover waits on it.
- The usage probe starts the CLI once per account per five minutes while that account has a run.

## Hot files

This work adds rows to the tables the hotspots plan (#246) turns into folders: methods, error codes, handlers,
commands, doctor rows. So the shed slices start after #248, #249 and #250, and then each row is a new file:
`crates/shed-proto/methods/account_ls.md`, `crates/shed-proto/errors/no_account.md`,
`crates/shed-server/src/methods/account_ls.rs`, `crates/shed/src/cmd/account.rs`,
`crates/shed/src/doctor/accounts.rs`. Those folders are collected and their old lists frozen by #248 to #250's
own `arch.toml` lines; this ADR adds none for them.

What this ADR adds:

- **Docs, per area.** A new folder `docs/accounts/`, one file per slice (`accounts.md`, `login.md`, `choice.md`,
  `usage.md`, `token-proxy.md`, `security.md`, `machines.md`), so no two slices edit one doc.
  `docs/acp/accounts.md` is deleted in slice 3; its expired-login paragraph moves to `docs/accounts/accounts.md`.
- **Records** stay one file per provider in `crates/shed-catalog/providers/`, collected by `build.rs` as today.
- **The new crate** has one file per concern, listed in [README.md](README.md), so the slices do not share one.

The `.drover/arch.toml` lines. Slice 1 adds:

```toml
[[collect]]
dir = "docs/accounts"
glob = "*.md"
```

Slice 2 adds, and puts `"accounts"` into the `server` layer's `may_use`:

```toml
[[layer]]
name = "accounts"
paths = ["crates/shed-accounts/src/**"]
mods = ["shed_accounts"]
may_use = ["core", "proto"]

[[collect]]
dir = "crates/shed-accounts/src"
glob = "*.rs"

[[ban]]
pattern = ".expose()"
paths = ["crates/*/src/**"]
except = ["crates/shed-accounts/src/env.rs", "crates/shed-accounts/src/proxy.rs"]
reason = "a token leaves its type only into a main-host run's environment or the proxy's upstream header"
```

Slice 5 adds:

```toml
[[ban]]
pattern = "TcpListener"
paths = ["crates/*/src/**"]
except = ["crates/shed-accounts/src/relay.rs"]
reason = "shed listens on Unix sockets; the one TCP listener is the relay, on loopback"
```
