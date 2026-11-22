# The security model

Three properties, each with what holds it. "Test" means a test the slice ships; "ban" means an
`.drover/arch.toml` line from [adr.md](adr.md).

## 1. A token never leaves the main host

| rule | held by |
|---|---|
| A token is a `Token`: no `Debug`, `Display`, `Serialize` or `Clone`. | the type; a compile-fail doc test |
| `Token::expose` is called in two files: `env.rs` (a run on the main host) and `proxy.rs` (the upstream header). | ban `.expose()` |
| No method returns a token or takes one. `account.add` takes a path on the main host; `account.ls` gives `TokenInfo`: two dates and a flag. | test: every method's result for an account with a token, searched for the token |
| A token file wider than 0600 is not read. | test; a doctor row names the file and the `chmod` |
| A `run` sent over a link carries no `account`, no folder and no token: only the relay's URL and a key. | test: a fake machine keeps every byte it is sent; none is the token |
| An account without a token, or of a provider without `[proxy]`, does not run on a machine. | `ACCOUNT_LOCAL_ONLY`; test |
| The token reaches `curl` in its config on stdin, never in an argument. | test: the runner's recorded arguments |
| The token is in no log, event, `get`, state file, or hibernated session. | the tests #191 wrote for `run`'s `env`, run again with an account |
| The keychain item and the credentials file are read only to hash them for the drift check: the bytes are dropped at once and appear in no log, event, line, state file or method result. | test: a fake `security` and a fake folder; every method result, the doctor's output and every file under shed's folder searched for both fake values |
| The login terminal is `redacted`: `read`, `screen.watch`, `wait --output` and the events pass through `redact`; `attach` is refused. | test per read path, with a token on the screen |
| With `token`, shed takes the setup token from the unredacted screen itself and writes the 0600 file. | test: a fake CLI prints a token; readers see `[redacted]`, the file holds it |

## 2. A per-run key is worth one run

| rule | held by |
|---|---|
| A key is 24 random bytes; `proxy-keys.json` holds its sha256. | test: the file searched for the key |
| It is made by `run` for one terminal, and no method makes one. | the API: there is no mint call |
| It ends when its terminal exits, is killed or is removed, and on `proxy.revoke`. No timer. | test per ending: the next call is 401 |
| It signs only paths under the record's `proxy.paths`, checked before the token file is opened. | test: a path outside is 400 and the token file's read count stays 0 |
| The proxy listens on a 0600 Unix socket on the main host, never on TCP. | ban `TcpListener` |
| The relay on a machine binds loopback, and only relays bytes: it holds no key and no token. | ban `TcpListener` outside `relay.rs`; test: its bind address |
| The proxy drops the caller's `authorization` and `x-api-key` before it adds its own. | test, moved from drover |
| An upstream 401 marks the token `rejected` once, not once per lane; the proxy then answers 401 itself until a new token is saved. | test, moved from drover's `proxy-token-rejected` |

A process on the machine that reads another run's environment can use that run's key until the run ends. That
is drover's exposure today; a machine is one trust zone. What it cannot get is the token, or a key that
outlives the run.

## 3. A lane never loads the owner's own setup

The owner's setup is their config folder: instructions, settings, hooks, MCP servers, skills. Today drover keeps
a lane out of it with session options (`meta`: setting sources, excludes, strict MCP), and two guards make sure
those options take effect. All of that stays, and the account layer adds to it:

| rule | held by |
|---|---|
| shed sets the account's variables and nothing else: `meta`, the adapter's command and the settings variable pass through untouched. | test: `session/new`'s `_meta` and the command are the same with and without `account` |
| A caller's `env` that sets an account variable (the folder, token, base URL or key variable) is refused. | `INVALID_PARAMS`; test |
| A named account's folder holds a login, not a setup: `account.add` makes it empty and refuses to adopt a folder holding one of the record's `setup_files`; every start checks again. | `ACCOUNT_SETUP`; test |
| `auto` never picks `default`, the owner's own folder. A caller that wants it names it. | test; open question 2 |
| A run on an account inherits none of the record's `scrub` and `api_vars` variables from shed's own environment, so an exported token or API key cannot replace the account. | test; this replaces drover's doctor warning |
| On a machine, shed sets the two proxy variables and no folder: the lane's guard there is the caller's `meta`, as now. | unchanged |

**drover's two guards still hold**, because both run before `run` and the account layer changes nothing they
look at:

- `ACP_UNGUARDED` checks that the lane's adapter runs drover's hook: the profile's settings, which drover puts
  in the worktree. `account` moves the config folder, not the worktree, and with drover's `meta` the adapter
  reads project and local settings only. drover adds one check of its own: it refuses an account whose `setup`
  list is not empty (shed refuses it too; drover's refusal names the role).
- `SHED_OLD` checks that the host's shed honours `meta`. Since #191 that is the client's protocol floor. This
  ADR raises drover's floor to 5 and adds the same rule for accounts: a machine whose shed is under protocol 5
  is `NO_PROXY`; shed never falls back to that machine's own login.

## What this does not cover

- The main host itself. A lane there can read its own environment, and its account folder: the folder variable
  has to point at it. A lane that must not hold its login runs on another machine.
- Another user on the main host with root.
- What the agent sends to its provider.
