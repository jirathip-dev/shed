# shed-client and the provider catalog (#191)

## Summary

Decision: shed gets two crates beside shed-proto. shed-client is the one socket client, blocking and with no async runtime: connect and version check, typed calls for terminals, ACP sessions, streams, agents and machines, one error type. shed-catalog holds the agent records (program, commands, models, model and effort words, limit and sign-in lines), built in and overridable per host and per app. shed's own CLI and server use both first; cooee and drover then delete their copies. One minimum protocol lives in the client, and from protocol 3 a server refuses a request field it does not know, so an old server fails loudly instead of dropping it.
Slices: (1) the client crate, with shed's CLI and server moved onto it; (2) protocol 3 and the one minimum; (3) the catalog crate and the records' new keys; (4) run takes a model, an effort and an environment; (5) a test server and the skill. Then four adoption issues: cooee client, cooee catalog, drover client, drover catalog. About +690 lines in shed; about 1,050 fewer in cooee and drover, and 424 fewer test lines.
Open questions, answered by the owner in q2648 (approved, with the 2026-11-20 amendment): (1) run takes a model, an effort and an environment, so shed builds the agent's command, and the environment's values never leave the server's memory (not in get, agents, events, logs or a hibernated session, each proven by a test). (2) One cap raise to 14,300, in its own cap issue, sized for these slices and the open PR already over. (3) The shed fleet's done check becomes just check.

The question, the options, the decision and its consequences are in [adr.md](adr.md). This file has how the
callers read, the shapes, the version contract, the boundaries, the statechart, the inventory, the
agent-speed mechanisms and the slices.

## How it reads

### cooee: start a chat's agent, prompt it, follow it

Today, condensed from `cooee-core/src/shed/` (887 lines: its own connection, pool, handshake and errors):

```rust
let cmd = shed.agents()?.into_iter().find(|a| a.id == agent).map(|a| a.cmd);   // runs `shed --json agents`
let mut launch = Launch { env, secrets: Some((file, vars)), mcp: Some((mcp_file, servers)), meta, policy, ..Launch::default() };
launch.pick(provider.picks(effort)?);     // cooee's own effort words; a NAME=value word becomes a variable
shed.run_acp(&name, (agent, &cmd), cwd, &labels, &launch)?;
//  = shed [--on m] --json run --acp <agent> --mcp <file> --meta <json> … -- \
//        env K=V … sh -c '<source the secrets file, remove it, exec "$@">' <file> <adapter…> <effort words>
shed.ready(&name)?;
shed.prompt(&name, &text)?;               // cooee's copy of the prompt framing
```

After:

```rust
use shed_client::{Calls, Error, Shed, Streamed, proto::{ErrorCode, EventsParams, RunParams}};
use shed_catalog::Catalog;

let shed = Shed::at(shed_client::default_socket(env_var)?);
shed.run(&RunParams {
    name: Some(name.clone()), acp: Some(agent.into()), cwd, labels, policy,
    model, effort,                        // shed turns them into the record's words, for the driver it picks
    env,                                  // the account's folder and the secrets: in memory, on no command line
    mcp_servers, meta, resume, machine,
    ..RunParams::default()
})?;
let session = shed.ready(&name, 60.0)?;
shed.prompt(&name, &text)?;

for item in shed.events(&EventsParams { labels: mine, since })? {
    match item {
        Streamed::Item(event) => hub.take(event),
        Streamed::Reset(seq) | Streamed::Lagged(seq) => hub.lost_up_to(seq),
        Streamed::Skipped(line) => note!("a shed event this build does not know: {line}"),
        Streamed::Quiet => {}
    }
}

match shed.get(&name) {
    Err(e) if e.code() == Some(ErrorCode::NoSuchTerminal) => thread.ended(),
    Err(e @ Error::Version { .. }) => doctor.row(e.to_string()),   // one text with its fix, written in shed-client
    other => thread.show(other?),
}
```

Its providers, today three layers merged by cooee (`provider.rs`: `specs`, `facts`, `finish`, and a
`defaults()` per module), after:

```rust
let served = Catalog::served(&shed.agents()?);        // the built-in records under the host's files
let catalog = served.over("codex", r#"login = "codex login --device-auth""#)?;   // a row of cooee's own config
let record = catalog.by_id(agent).ok_or(NoAgent)?;    // login, status, logout, config_dir_var, token_var, install
let persona = providers::of(&record.agent).session(&want);   // cooee's own: what a persona session is told
```

### drover: start a lane on this host or another, kick it

Today, condensed from `core/src/shed.rs`, `term.rs` and `spawn.rs`:

```rust
if !shed_meta(r) { return refused("SHED_OLD", …) }            // reads `shed help run` for `--meta`
let adapter = shed::adapter(r, id)?;                          // `shed agents --json`, two shapes
let env = providers::get(&p.harness).acp_env(model, effort);  // drover's own model and effort facts
shed::cli(r, &self.acp_argv(name, labels, cwd, &acp))?;       // `shed run --acp … --mcp <file> --meta … -- sh -c '<line>' sh <adapter…>`
self.call(r, Method::Wait, WaitParams { children: None, kind: None, name: name.into(), until: vec![],
    output: Some("ACP session ready".into()), timeout: Some(60.0) })?;
shed::cli(r, &["send", name, text])?;                         // the CLI, for its prompt framing
```

After. The typed calls are one trait, so they are the same on a pooled socket and through drover's runner:

```rust
struct Remote<'a>(&'a dyn Runner);
impl Calls for Remote<'_> {
    fn call_raw(&self, method: Method, params: Value) -> Result<Value, Error> {
        shed_client::once(method, &params, |lines| connect(self.0, lines))   // `ssh host shed connect`, a call each
    }
}
let (local, remote) = (Shed::at(&socket), Remote(r));
let shed: &dyn Calls = if wire == Wire::Connect { &remote } else { &local };

let adapter = installed(&shed.agents()?, id)?;        // typed; drover's prologue still wraps it (adr.md, "left to the apps")
shed.run(&RunParams { name: Some(lane.into()), acp: Some(id.into()), policy: Some(Policy::Allow),
    model, effort, cmd: [prologue, adapter].concat(), mcp_servers, meta, no_hibernate, labels, cwd,
    ..RunParams::default() })?;
shed.ready(lane, 60.0)?;
shed.prompt(lane, kick)?;
shed.wait_state(lane, &["working", "blocked", "done"], 20.0)?;
```

Its config check needs no server, and its doctor row is the server's hello against the client's floor:

```rust
let catalog = Catalog::built_in();
let efforts = &catalog.by_agent(&role.harness).ok_or(NoHarness)?.efforts;   // was EFFORTS and each module's list
let fix = catalog.by_id(id).and_then(|p| p.login.as_deref());              // was `login()` in three modules
format!("✓ shed — server {}, drover needs protocol {MIN_PROTOCOL} or newer", label(&local.hello()?))
```

### shed itself, and a test in any of the three repos

```rust
let done: WaitResult = again(&server, |shed| shed.wait(&params))?;   // the CLI keeps its retry sleep and autostart

let fake = shed_client::fake::Server::start();                       // a real socket in a temp folder, removed on drop
fake.answer(Method::Get, json!(terminal("t1", "working")));
assert_eq!(my_code(&Shed::at(fake.socket())).state, "working");
assert_eq!(fake.seen(), [(Method::Get, json!({"name": "t1"}))]);
```

## Data shapes

### shed-client

```rust
/// The oldest server protocol this client works against; what `hello` carries as `oldest`.
pub const MIN_PROTOCOL: u32 = 3;
pub use shed_proto as proto;

/// A server: its socket, at most four idle connections, a call timeout (10 s).
#[derive(Clone)]
pub struct Shed { /* socket, pool, timeout, tap */ }
impl Shed {
    pub fn at(socket: impl Into<PathBuf>) -> Self;
    pub fn timeout(self, call: Duration) -> Self;
    pub fn tap(self, tap: Tap) -> Self;                  // called around each call: an app's timing, with its own clock
    pub fn hello(&self) -> Result<Hello, Error>;         // the server's hello, floor checked
    pub fn connect(&self) -> Result<Conn, Error>;        // a connection of your own
    pub fn events(&self, params: &EventsParams) -> Result<Stream<Event>, Error>;
    pub fn watch(&self, name: &str, since: Option<u64>) -> Result<Stream<ScreenUpdate>, Error>;
    pub fn attach(&self, name: &str) -> Result<(BufReader<UnixStream>, UnixStream), Error>;
}

/// One connection, past the handshake.
pub struct Conn { /* reader, writer, next id, the server's hello */ }
impl Conn {
    pub fn over(stream: UnixStream) -> Result<Self, Error>;   // a socket, or a socketpair to `ssh … shed connect`
    pub fn server(&self) -> &Hello;
    pub fn call<R: DeserializeOwned>(&mut self, method: Method, params: &impl Serialize) -> Result<R, Error>;
}

/// The typed calls, written once over `call_raw`: every method under it is provided. `Shed` implements
/// the trait; so can any transport.
pub trait Calls {
    fn call_raw(&self, method: Method, params: Value) -> Result<Value, Error>;
    // terminals.rs
    fn run(&self, params: &RunParams) -> Result<TerminalInfo, Error>;
    fn ls(&self, params: &LsParams) -> Result<Vec<TerminalInfo>, Error>;
    fn get(&self, name: &str) -> Result<TerminalInfo, Error>;
    fn read(&self, params: &ReadParams) -> Result<ReadResult, Error>;
    fn screen(&self, name: &str) -> Result<String, Error>;
    fn send(&self, params: &SendParams) -> Result<(), Error>;
    fn keys(&self, name: &str, keys: &[&str]) -> Result<(), Error>;
    fn resize(&self, name: &str, rows: u16, cols: u16) -> Result<(), Error>;
    fn state(&self, name: &str, report: &Reported) -> Result<TerminalInfo, Error>;
    fn kill(&self, name: &str) -> Result<TerminalInfo, Error>;
    fn rm(&self, name: &str) -> Result<TerminalInfo, Error>;
    fn close(&self, name: &str) -> Result<(), Error>;            // kill, then rm; already gone is fine
    fn wait(&self, params: &WaitParams) -> Result<WaitResult, Error>;
    fn wait_state(&self, name: &str, until: &[&str], secs: f64) -> Result<WaitResult, Error>;
    fn wait_output(&self, name: &str, pattern: &str, secs: f64) -> Result<WaitResult, Error>;
    // acp.rs
    fn ready(&self, name: &str, secs: f64) -> Result<AcpSession, Error>;
    fn prompt(&self, name: &str, text: &str) -> Result<(), Error>;   // `working`, then the framed input
    fn approve(&self, name: &str, option: &str, request: Option<&str>) -> Result<(), Error>;
    fn cancel(&self, name: &str) -> Result<(), Error>;
    fn model(&self, name: &str, model: &str) -> Result<(), Error>;
    // agents.rs, machines.rs
    fn agents(&self) -> Result<Vec<AgentInfo>, Error>;
    fn matched(&self, id: &str, text: &str) -> Result<Matched, Error>;
    fn links(&self) -> Result<BTreeMap<String, Link>, Error>;
    fn exec(&self, params: &ExecParams) -> Result<ExecResult, Error>;
    fn stats(&self, params: &StatsParams) -> Result<MachineStats, Error>;
    // and one provided method for each `Method` left: `file.read`, `file.write`, `emit`, `handover.check`, `handover`
}

/// One call on a connection that lives for one exchange: the lines to send in, every line that came back out.
pub fn once<R: DeserializeOwned>(method: Method, params: &impl Serialize,
    exchange: impl FnOnce(&str) -> io::Result<String>) -> Result<R, Error>;
pub fn notification(method: Method, params: &impl Serialize) -> Result<Vec<u8>, Error>;
/// The per-user socket; `var` is the caller's own way to read a variable.
pub fn default_socket(var: impl Fn(&str) -> Option<OsString>) -> Result<PathBuf, Error>;

/// A stream on a connection of its own; `tick` makes a silent stretch an item, so the caller can look up.
pub struct Stream<T> { /* … */ }
impl<T> Stream<T> { pub fn tick(self, every: Duration) -> Self; pub fn since(&self) -> Option<u64>; }
impl<T: DeserializeOwned> Iterator for Stream<T> { type Item = Streamed<T>; }
pub enum Streamed<T> { Item(T), Reset(u64), Lagged(u64), Quiet, Skipped(String) }

#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Nothing listens on the socket, or the connection broke.
    Io { socket: PathBuf, source: io::Error },
    /// Not shed, a server under `MIN_PROTOCOL`, a refused hello, or a call this server is too old for.
    Version { ours: Hello, theirs: Option<Hello>, needs: Option<String> },
    /// The server's own error.
    Rpc(RpcError),
    /// An answer that could not be read.
    Bad(String),
}
impl Error { pub fn code(&self) -> Option<ErrorCode>; }
```

`Version` prints one line, the same for every app, and names the side to update as the server's own refusal
does:
`SHED_VERSION: this client speaks shed protocol 3 to 3; the server is 0.0.0 (abc1234) protocol 2; update shed there (it updates itself from its own releases)`,
and with `needs`: `…; the server does not know run.env; update shed there (…)`.

### shed-proto, each in a new file (`agents.rs`, `input.rs`), never in `lib.rs`

```rust
// Provider: the new keys
/// The command that installs the program.
pub install: Option<String>,
/// The variable that carries a login token.
pub token_var: Option<String>,
/// The models a start may name: id and name (moved up from the stream table).
pub models: Vec<[String; 2]>,
/// The adapter's arguments that set the model, `{model}` standing for the pick.
pub model_args: Vec<String>,
/// The variable that sets the model, where the adapter takes one.
pub model_env: Option<String>,
// Stream: `effort: Vec<String>`, beside its `model`.

impl Provider {
    /// What one start on `driver` adds for a model and an effort; an effort it does not take is the error.
    pub fn pick(&self, driver: DriverKind, model: Option<&str>, effort: Option<&str>) -> Result<Picked, String>;
}
pub struct Picked { pub env: Vec<(String, String)>, pub args: Vec<String> }

// RunParams: the new fields (slice 4), and `#[derive(Default)]` on every params type (slice 1)
pub model: Option<String>,
pub effort: Option<String>,
/// Variables for the program, over the server's own. Kept in memory only; never saved or shown.
pub env: BTreeMap<String, String>,

// Moved from shed-server, unchanged: `Input`, `frame`. New: `ErrorCode::of(code: i64) -> Option<ErrorCode>`.
```

A record after slice 3, the new keys only (`crates/shed-catalog/providers/claude.toml`; the values are
today's from the two apps, and where they differ the adoption lane picks what the real program does and says so in its PR):

```toml
install = "curl -fsSL https://claude.ai/install.sh | bash"
token_var = "CLAUDE_CODE_OAUTH_TOKEN"
models = [["opus", "Opus"], ["fable", "Fable"], ["sonnet", "Sonnet"], ["haiku", "Haiku"]]
model_env = "ANTHROPIC_MODEL"

[stream]
effort = ["--effort", "{effort}"]
```

### shed-catalog

```rust
pub struct Catalog { /* entries, by agent */ }
pub struct Entry { pub provider: Provider, pub source: String }   // `built-in`, a file's path, or `app`
impl Catalog {
    pub fn built_in() -> Self;                              // the records compiled in; no server, no file
    pub fn load(dir: &Path) -> (Self, Vec<String>);         // under `<config>/providers/*.toml`; and the bad files
    pub fn served(agents: &[AgentInfo]) -> Self;            // a server's answer to `agents`
    pub fn over(self, agent: &str, toml: &str) -> Result<Self, String>;   // an app's row: keys over that record, or a new one
    pub fn by_id(&self, id: &str) -> Option<&Provider>;     // the ACP id
    pub fn by_agent(&self, agent: &str) -> Option<&Provider>;   // the program's name, which is the file's name
    pub fn iter(&self) -> impl Iterator<Item = &Entry>;
}
```

```
 built-in records          crates/shed-catalog/providers/<agent>.toml, compiled in
        ▲ key by key
 the host's files          <config>/providers/<agent>.toml: every app on that host sees them (`agents`)
        ▲ key by key
 an app's rows             `Catalog::over`, from the app's own config: that app only
```

One merge rule for both upper layers: a table of keys over the record of that name, checked as a whole
record afterwards; an unknown key is an error naming the file or the row.

## The version contract

| the server | what the client's caller gets |
|---|---|
| is not shed, or sends no protocol | `Version` at connect |
| speaks a protocol under 3 | `Version` at connect: its own refusal of the range, or the client's of its answer |
| is at 3 or above and lacks a method or field the call uses | `Version` on that call, naming it; every other call works |
| is at 3 or above and knows the call | the answer |
| is newer, with a breaking protocol, and still serves the client's | the answer, in the client's shapes |
| is newer and no longer serves the client's | `Version` at connect: update the app |

Three things keep it true: the server test "a field the method does not document is refused on protocol 3
and taken on 2"; the client tests against the fake server for each row; and `just protocol`, which compares
the base's `docs/shed.openrpc.json` with the head's and fails when a method, a field or a type is gone
while `PROTOCOL` (written into the document's `info`) is the same. One fixture breaks it.

## Module boundaries

```
  shed (CLI) ───▶ shed-server ───▶ shed-core
      │             │      │
      │             │      └─────▶ shed-catalog ──┐
      ▼             ▼                             ▼
  shed-client ──────────────────────────────▶ shed-proto
      ▲                                           ▲
  cooee, drover ──▶ shed-client and shed-catalog, at one git rev; shed-proto comes with them
```

| crate | file | holds | may use |
|---|---|---|---|
| shed-proto | `agents.rs` | `Provider` and its tables, `Picked`, `pick`, the readers (`limit_hit`, `auth_failure`, `tokens`, `cache_lifetime`), `Agent`, `AgentInfo`, `Matched` | nothing |
| | `input.rs` | `Input`, `frame` | nothing |
| shed-client | `conn.rs` | `Conn`, the handshake, the floor, `once`, `notification`, the `Version` text | proto |
| | `shed.rs` | `Shed`, the pool, timeouts, `default_socket`, `tap` | `conn` |
| | `terminals.rs`, `acp.rs`, `agents.rs`, `machines.rs` | the provided methods of `Calls`, by domain | `conn` |
| | `stream.rs` | `Stream`, `Streamed`, `events`, `watch`, `attach` | `conn`, `shed` |
| | `fake.rs` (feature `fake`) | `fake::Server` | proto only: it shares no code with `conn`, so a client bug cannot hide in it |
| shed-catalog | `lib.rs`, `build.rs`, `providers/*.toml` | the records, `Catalog` | proto |
| shed-server | `providers.rs` | `known` (installed agents, medians), `wire` | catalog |
| | `providers/*.rs` | the stream driver's line formats | as today |
| | `drivers/launch.rs` (slice 4) | `pick` and `env` applied to the chosen driver's command | proto |
| shed | `main.rs` and its commands | `Shed` for every call; autostart, the retry sleep, `skill` | server, client |

Rules: the client and the catalog never use each other, the core or the server. Neither reads a clock or the
environment or starts a process (the existing bans, no exception). An agent's name appears only in
`crates/shed-catalog/providers/` and `crates/shed-server/src/providers/` (`just neutral`, its rule line
updated). Nothing but the client and the server's dispatch builds a `hello` (a new ban, [adr.md](adr.md)).

## Statechart: a connection

```
              dial                      hello answered, protocol ≥ MIN_PROTOCOL
  (none) ─────────────▶ dialing ─────────────────────────────────────────────▶ idle, in the pool (≤ 4)
                          │ nothing listens ─────────────▶ Io                    │        ▲
                          │ not shed, under the floor,                           │ take   │ result, or the
                          │ or the server refused ───────▶ Version               ▼        │ server's own error
                                                                              in a call ──┘
        taken from the pool with its peer gone (a restart, a handover):          │ Io or Bad
        dropped, dial again, no error                                            ▼
                                                                             dropped: the error goes to the
                                                                             caller; the request is never sent
                                                                             twice (the server may have acted)

  a stream:  dial ──▶ subscribed ──▶ streaming ──▶ ended (the peer closed)
             items: Item | Reset(seq) | Lagged(seq) | Quiet (a tick passed) | Skipped (a line it cannot read)
```

A `wait` is a call whose deadline is its own timeout plus the call timeout, or none. A stream never returns
to the pool. Reconnecting a stream is the caller's loop (`Stream::since` is where to resume), because the
pause between tries needs a clock.

## Inventory

**shed**

| today | becomes |
|---|---|
| `shed-server/src/lib.rs`: `Client`, `notification` (95) | `shed_client::{Conn, Shed, notification}`; deleted |
| `shed-server/src/lane.rs`: `Input`, `frame` | moved to `shed_proto` |
| `shed-server/src/providers.rs`: `load`, `find`, `parse`, `record` (60) | `shed_catalog::Catalog`; deleted |
| `shed-server/src/providers.rs`: `known`, the timings and medians | kept |
| `shed-server/src/providers/*.toml`, the records half of `build.rs` | moved to `crates/shed-catalog/` |
| `shed-server/src/providers/{claude,plain}.rs`, the wires half of `build.rs` | kept |
| `shed-server/src/machines.rs`: `connect` | kept, on `Conn::over` |
| `shed/src/main.rs`: `connect`, `again`, `send`, `to_acp`, `links`, `events` | call `Shed`; autostart and the retry sleep kept |

**cooee** (`crates/cooee-core/src/`)

| today | becomes |
|---|---|
| `shed/client.rs` (247): `Shed`, `Conn`, pool, handshake, `cli` | `Shed`, `Conn`; deleted |
| `shed/error.rs` (67): `ShedError`, `SHED_MIN_PROTOCOL`, `shed_fix` | `Error`, `MIN_PROTOCOL`, the `Version` text; deleted |
| `shed/terminals.rs` (166): `get`, `close`, `run`, `ls`, `resize`, `screen`, `type_line`, `exited`, `watch` | `Calls`, `Shed::watch`; deleted |
| `shed/acp.rs` (233): `ready`, `prompt`, `approve`, `cancel`, `set_model`, `events`, `Streamed` | `Calls`, `Shed::events`; deleted |
| `shed/acp.rs`: `agents`, `records` | `Calls::agents`, `Catalog::served`; deleted |
| `shed/acp.rs`: `run_acp` (the CLI words, the secrets file, the MCP file) | `Calls::run` with `env`, `model`, `effort`, `mcp_servers`; deleted |
| `shed/machines.rs` (64): `link`, `links` | `Calls::links`; `run_on` kept, on `run`, `wait` and `close` |
| `shed/launch.rs` (110): `Record`, `Listed` | `proto::{Provider, AgentInfo}`; deleted. `Agent` (its models) kept; `Launch` shrinks to what a persona adds |
| `provider.rs` (345): `specs`, `facts`, `finish`, `Entry::{defaults, keep}`, `defaults` | `Catalog::served(…).over(…)`; deleted |
| `provider.rs`: the `Provider` trait, `Want`, `Setup`, `Gap`, `Builtin`, `Channel` | kept: what a persona session is told is cooee's |
| `providers/{claude,codex,gemini}.rs`: `defaults()`, the token variable, `keep: ["login"]` | the records (gemini's is new); the login choice is one app row |
| `providers/claude.rs`: `read_status`, `read_usage`; every module's `session` | kept (cooee only) |
| `providers/fake.rs` and the fake `shed` scripts of its tests | the provider is an app row in the test; the server is `shed_client::fake` |

**drover** (`crates/core/src/`)

| today | becomes |
|---|---|
| `shed.rs` (349): `MIN_PROTOCOL`, `UPDATE`, `lines`, `short`, `side`, `accepted`, `answer`, `hello`, `call`, `exchange` | `Shed`, `once`, `MIN_PROTOCOL`, the `Version` text; deleted |
| `shed.rs`: `events`, `follow`, `adapter`, `default_socket` | `Shed::events` with `tick`, `Calls::agents`, `default_socket`; deleted |
| `shed.rs`: `cli` | deleted once `send` and `cancel` are `prompt` and `cancel` |
| `shed.rs`: `Wire`, `Shed` (the runtime's settings), `With`, `line`, `builds`, `ready`, `version_alert`, `connect`, `unit`, `start_job`, `seen` | kept: drover's hosts, doctor rows, refusals and alerts, over the client's errors |
| `term.rs`: `acp_argv`, `kick_acp`, each `WaitParams { … }` literal | `run`, `ready`, `prompt`, `wait_state`, `wait_output`; the pane-word mapping kept |
| `spawn.rs`: `shed_meta`, `SHED_OLD` | deleted: the floor |
| `providers/mod.rs` (330): `acp`, `login`, `install`, `efforts`, `EFFORTS`, `acp_env`, `token_env`, `health_check` | the record's `id`, `login`, `install`, `efforts`, `token_var`, `status`, and `model`/`effort` on `run`; deleted |
| `providers/mod.rs`: the registry, `Surface`, `Dialect`, `Proxy`, `Needs`, `sync`, `ready`, `prepare`, `start_cmd`, `kick`, `vendor`, `brands` | kept: what drover does with an agent |
| `providers/claude.rs` (214), `codex.rs` (87), `gemini.rs` (19) | the facts above go; the hook dialect, proxy, sandbox, profile sync and terminal launch stay |
| its two terminal-only harness modules (411), `providers/fake.rs` (104) | kept: shed has no record for them |
| `crates/cli/tests/common/fake_shed.rs` (424) | `shed_client::fake::Server` |

## The agent-speed criteria

| # | criterion | mechanism |
|---|---|---|
| 1 | one gate, same everywhere | `just check` is the gate. CI's gate job becomes one step, `just check`, so the two lists cannot drift; the shed fleet's `done_checks` is `just check` (open question 3, accepted). Both crates join through `[lints] workspace = true`, the layers and bans in `arch.toml`, the cap and `deny`, so they are under every gate from their first commit. What stays CI-only is named: the other OS's test leg and the base-against-head benches. |
| 2 | a skill | `skill/shed-client/`: `SKILL.md` (connect, the calls by domain, errors, the version contract, the catalog's layers) and `examples/` (`lane.rs`, `events.rs`, `catalog.rs`, `fake.rs`). `shed skill --to <dir>` writes it from the binary (collected by the build script, as the records are). The examples are `shed-client`'s cargo examples, so `just clippy` and `just test` compile them and the skill cannot go stale. |
| 3 | fix once | The inventory above; shed's own CLI and server run on the same client, so a client bug shows in shed's tests first. Each adoption PR adds a `Hello::ours` ban to its repo's `arch.toml`, so a second client cannot come back. |
| 4 | smaller blast radius | cooee about −800 lines, drover about −250 and −424 of tests. In shed: two crates on their own layers; the typed calls one file per domain; one file per record; the server's `lib.rs` 95 lines shorter; a new `RunParams` field breaks no app (`Default`). |
| 5 | fast | Each crate states `[package.metadata.gate] test_secs` in its own `Cargo.toml` (client 10, catalog 5; the four existing crates twice their measured time). `just budget`, part of `just check`, builds the tests, times each crate's run and fails over budget; a fixture crate with a budget of 0 proves it bites. The client has no async runtime and its tests use the fake server, no PTY. |

"As strict as possible", for the two crates: pedantic clippy and the deny lints by the workspace table;
`cargo deny` (their only new dependency edge is `toml`, already in the tree); the line cap; the comment
rule; two layer fixtures (the client using the server, the catalog using the client) and one ban
fixture for `Hello::ours`; one fixture each for `just protocol` and `just budget`.

## Slices

| # | what | main files | src lines |
|---|---|---|---|
| 1 | The client; shed's CLI and server on it; `Default` on params; `Input` to proto; CI's gate is `just check`; `just budget` | `crates/shed-client/`, `shed-proto/src/input.rs`, `shed-server/src/{lib,links,machines,lane}.rs`, `shed/src/`, `justfile`, `ci.yml`, `arch.toml` | about +335 |
| 2 | Protocol 3: unknown request fields refused; `MIN_PROTOCOL`; `just protocol` | `shed-server/src/lib.rs` (dispatch), `shed-client/src/conn.rs`, `scripts/protocol.py` | about +40 |
| 3 | The catalog; the records move; new keys and `pick`; the gemini record | `crates/shed-catalog/`, `shed-proto/src/agents.rs`, `shed-server/src/providers.rs` | about +85 |
| 4 | `run` takes `model`, `effort`, `env` (open question 1); one test each that `env`'s values are in no `get`, `agents`, `events`, log or hibernated session | `shed-proto` (`RunParams`), `shed-server/src/drivers/launch.rs`, `shed/src/main.rs` (`--model`, `--effort`, `--env`) | about +50 |
| 5 | The fake server; the skill; `shed skill --to` | `shed-client/src/fake.rs`, `skill/shed-client/`, `shed/src/main.rs` | about +180 |

Each slice is one PR, serial, in this order; 1 and 3 are moves plus a typed surface, reviewed with
`git diff --color-moved`. The numbers are estimates from the code that moves and the copies it replaces.

Adoption, filed after this ADR is approved and started once a shed release with slices 1 to 5 is on every
host and every host's shed reports protocol 3:

1. **cooee, the client.** First, because `Shed` is cooee's pooled design made general, so it proves the API
   with the least change. Deletes `shed/` but `Agent`, `Launch` and `run_on`.
2. **cooee, the catalog.** `provider.rs`'s merge and the `defaults()` go; its rows become `Catalog::over`.
3. **drover, the client.** `Remote` over its runner, `fake::Server` in its tests, the floor in place of its
   probes.
4. **drover, the catalog.** The launch facts leave its provider modules.

Each adoption issue carries the five criteria; its PR states the lines it deleted against this inventory.
