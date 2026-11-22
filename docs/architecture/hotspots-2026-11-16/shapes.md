# Shapes: the row files, the collected tables and the layouts

## Collected folders

A collected folder holds one file per row. A build script lists the folder, sorted by file name, and writes a
table to `OUT_DIR`, which one source file includes. shed already does this twice: `shed-catalog/providers/*.toml`
and `shed-server/src/providers/*.rs`.

| folder | a row is | collected by | the table it writes |
|---|---|---|---|
| `crates/shed-proto/methods/*.md` | a method | `shed-proto/build.rs` | `Method`: the enum, `ALL`, `name`, `summary`, `route`, `input`, `needs_hello`, `schemas`, `stream` |
| `crates/shed-proto/errors/*.md` | an error code | `shed-proto/build.rs` | `ErrorCode`: the enum, `ALL`, `code`, `name` |
| `crates/shed-server/src/methods/*.rs` | a method's handler | `shed-server/build.rs` | the `mod` lines and one `match` from `Method` to `call` |
| `crates/shed/src/cmd/*.rs` | a command | `shed/build.rs` (new) | the `mod` lines, `Command`, its `on` and `run` matches |
| `crates/shed/src/doctor/*.rs` | a doctor row | `shed/build.rs` | the `mod` lines and the list of `rows` functions |
| `crates/shed/tests/fake_acp/*.py` | a prompt of the fake agent | the fake itself, by file name | nothing: it loads `<first word>.py` |
| `docs/commands/*.md` | a README row | `scripts/commands.py` (new) | the table in `README.md` |
| `crates/*/tests/*.rs`, `tests/<area>/` | a test binary, a topic | Cargo; `mod` lines in `<area>/main.rs` | nothing |
| `ci/fixtures/arch-*/` | a case arch-check must reject | the loop in `just arch` | nothing |

The build scripts share one helper file, `build/collect.rs` at the repo root, each through
`#[path = "../../build/collect.rs"] mod collect;`: the sorted file list with `cargo:rerun-if-changed`, the
`#[path] mod` lines, and stem to `CamelCase`. The two that collect today move onto it. Build scripts are outside
`crates/*/src`, so the line cap does not count them: about 220 lines after, 80 today.

The names join the crates: the file stem is the variant in snake case, everywhere. `methods/read_only.md`,
`src/methods/read_only.rs` and `Method::ReadOnly` are one method. No variant is renamed, because cooee and drover
name `Method::Input` and `Method::Model` at their pinned revisions.

## A method file

`crates/shed-proto/methods/code_outline.md`: `key: value` lines, a blank line, then the prose.

```
wire: code.outline
params: OutlineParams
result: OutlineResult
on: machine

A file's functions, types, sections and blocks in file order, each with its kind, name, lines and depth, by
line rules for Rust, TypeScript/JavaScript, Python, Markdown and TOML and by indentation for other files.
Over 2,000 symbols or 256 MiB is `LIMIT`.
`machine` reads it there.
```

| key | value | default | today it is |
|---|---|---|---|
| `wire` | the wire name | required | an arm of `Method::name` |
| `params`, `result` | a type of `shed_proto`, as written in Rust (`Vec<TerminalInfo>`) | `Empty` | an arm of `schemas` |
| `stream` | the type of the lines streamed after the response | none | an arm of `stream` |
| `schema` | a function over the schema pair (`terminal_for_name`) | none | a call in `schemas` |
| `on` | `machine` or `name`: the field that names the machine a held link carries the request to | none | an arm of `links::remote` |
| `input` | `yes`: refused on a connection that marked the terminal read-only | no | the list in `readonly` |
| `hello` | `no`: answered before the handshake | yes | two `if`s in `dispatch` |

The prose joined by single spaces is `Method::summary`, word for word today's. One sentence a line, so two PRs
that each add a sentence change different lines. `ALL` is in wire-name order, which is the order of the methods
in `docs/protocol.md` and `docs/shed.openrpc.json`.

## An error file

`crates/shed-proto/errors/no_such_terminal.md`:

```
name: NO_SUCH_TERMINAL
code: -32002

No terminal has that name.
```

`ALL` is in code order from -32000 down: shed's own codes, then JSON-RPC's. Two files with one code or one name
fail the build.

## What the proto build writes

```rust
pub enum Method { #[doc = "`run`: Start a terminal."] Run, /* one per file, in wire order */ }

impl Method {
    pub const ALL: [Self; 33];
    pub fn name(self) -> &'static str;
    pub fn summary(self) -> &'static str;
    pub fn route(self) -> Option<Route>;   // new: from `on`
    pub fn input(self) -> bool;            // new: from `input`
    pub fn needs_hello(self) -> bool;      // new: from `hello`
    fn schemas(self, g: &mut SchemaGenerator) -> [Schema; 2];   // feature `schema`
    fn stream(self, g: &mut SchemaGenerator) -> Option<Schema>; // feature `schema`
}

pub enum Route { Machine, Name }   // hand-written, in method.rs: the `machine` field, or the `<machine>/` in `name`

pub enum ErrorCode { /* one per file, in code order */ }
impl ErrorCode { pub const ALL: [Self; 17]; pub fn code(self) -> i64; pub fn name(self) -> &'static str; }
```

`Method::parse`, `openrpc`, `describe`, `Empty` and `terminal_for_name` stay hand-written in `method.rs`. The
writer keeps each arm on one line and merges arms with the same body, so the generated code passes pedantic
clippy with no `allow` (clippy does lint it: seen on a throwaway crate).

## A handler

`crates/shed-server/src/methods/kill.rs`:

```rust
use crate::call::{Call, Reply, json};
use crate::proto::Target;

pub(super) async fn call(cx: &mut Call<'_>) -> Reply {
    let Target { name } = cx.params()?;
    json(&crate::serve::kill(cx.registry, &name).await?)
}
```

```rust
pub(crate) type Reply = Result<Value, RpcError>;

pub(crate) struct Call<'a> {
    pub method: Method,
    pub request: &'a Request,
    pub registry: &'a Shared,
    pub session: &'a mut Session,   // protocol, beats, stream, watching, readonly, links, busy
}

impl Call<'_> {
    pub fn params<T: DeserializeOwned>(&self) -> Result<T, RpcError>;
    pub fn reg(&self) -> Result<MutexGuard<'_, Registry>, RpcError>;   // locked and refreshed
}
```

A handler that awaits nothing is a plain `fn call`; the build script writes `.await` after the ones declared
`async fn call`. The written `match` has no `_` arm. `dispatch.rs` keeps the gates in today's order, each reading
the method's own facts: handshake (`needs_hello`), read-only (`input`), unknown fields on protocol 3, the link
(`route`), then the handler. `hello`, `heartbeat`, `ls` (which merges the machines' lists) and `machine.links`
become ordinary handlers.

## A command, a doctor row

`crates/shed/src/cmd/kill.rs`:

```rust
use crate::cx::{Cx, On};
use shed_server::runner::ExitCode;

pub const ON: On = On::Link;

#[derive(clap::Args)]
pub struct Args {
    name: String,
}

pub fn run(cx: &Cx<'_>, args: Args) -> anyhow::Result<ExitCode> {
    let info = cx.server.connect(None)?.kill(&cx.server.name(args.name))?;
    cx.print(&info, || format!("{} exited\n", info.name))?;
    Ok(ExitCode::SUCCESS)
}
```

```rust
pub enum On { No, Ssh, Link }   // refuses --on; reaches the machine over ssh; goes over the held link when it is up

pub struct Cx<'a> { pub paths: &'a Paths, pub dir: Option<&'a Path>, pub json: bool, pub server: Server<'a> }

pub struct Row { pub mark: Mark, pub what: &'static str, pub text: String }
pub enum Mark { Ok, Fail, Note }   // `shed doctor` fails when any row is Fail
```

The build writes `enum Command { Kill(Box<kill::Args>), … }` with clap's `Subcommand` derive, `on(&Command) -> On`
and `run(&Cx, Command)`. `Server::new` then matches on no command: its three lists (`local`, `linkable`,
`terminal`) are each command's `ON` and its own `cx.server.name(…)` call. Nested subcommands (`machine`, `agents`,
`code`) and the hidden `acp-bridge` keep their attributes on `Args` (run on a throwaway crate with clap 4.6.7).
A doctor row is `src/doctor/<nn>_<row>.rs` with `pub(super) fn rows(cx: &Checks<'_>) -> Vec<Row>`; rows print in
file-name order, and the two digits keep today's order (`10_job`, `20_loaded`, `30_socket`, …).

## Tests, the fake agent, the goldens

`crates/shed/tests/acp.rs` becomes `tests/acp/main.rs` (the helpers and the `mod` lines; Cargo still builds one
binary named `acp`) plus one file per topic. Each `#[test]` moves whole. The first cut:

| area | topics |
|---|---|
| `acp` | `golden`, `turn`, `login`, `resume`, `usage`, `memory`, `hibernate`, `mcp`, `model`, `warm`, `restart`, `install` |
| `cli` | `golden`, `hello`, `state`, `wait`, `attach`, `serve`, `family`, `resources`, `skill` |
| `server` | `golden`, `protocol`, `events`, `screen`, `input` |
| `machines` | `golden`, `connect`, `link`, `acp`, `stats`, `exec` |
| `reliability` | `kill`, `lock`, `requests`, `files`, `signals` |
| `service` | `install`, `doctor`, `autostart` |

Every test that writes a file under `tests/golden/` lives in its area's `golden.rs`. `just generate` gains

```
cargo metadata --format-version 1 > /dev/null
python3 -I "{{justfile_directory()}}/scripts/commands.py"
SHED_BLESS=1 cargo test -p shed --locked --test cli --test acp --test server --test machines golden::
```

The fake agent keeps its handshake, flags and helpers in `fake_acp.py`. Each prompt is
`fake_acp/<word>.py` with `def run(agent, text)` returning the stop reason and its paragraph of today's docstring;
the core loads the file named by the prompt's first word, else `fake_acp/echo.py`.

## Docs

`docs/commands/run.md`: the first paragraph is the Command cell, the rest joined is the What-it-does cell.
`scripts/commands.py` (stdlib, run with `-I`, like `scripts/changelog.py`) rewrites the rows between
`<!-- commands -->` and `<!-- /commands -->` in `README.md`, in file-name order, and is idempotent.

`docs/acp/`: `README.md` (today's first six lines), `install.md`, `run.md`, `lane-states.md`,
`provider-records.md`, `hibernation.md`, `warm-sessions.md`, `prompt-cache.md`, `mcp.md`, `session-options.md`,
`stream-driver.md`, `how-it-works.md`, `v2.md`, `accounts.md`, and `smoke/<date>-<what>.md` for the manual smoke
and each dated result. Text moves word for word; the four files that link `docs/acp.md` are repointed.

## Layouts by concern

| crate | file | holds |
|---|---|---|
| shed-proto | `lib.rs` | `VERSION`, `BUILD`, `PROTOCOL`, `JSONRPC`, the modules, `pub use <module>::*` |
| | `envelope.rs` | `Request`, `Response`, `RpcError`, `Hello` |
| | `method.rs` | the included tables, `Route`, `parse`, `openrpc`, `describe`, `Empty`, `terminal_for_name` |
| | `terminal.rs` | `RunParams`, `Parent`, `Policy`, `LsParams`, `Target`, `ReadParams`, `Cursor`, `ReadResult`, `Logged`, `SendParams`, `KeysParams`, `ResizeParams`, `StateParams`, `Reported`, `WaitParams`, `Children`, `WaitOutcome`, `WaitResult`, `State`, `TerminalInfo`, `Resources` |
| | `events.rs` | `EventsParams`, `Event`, `EventKind`, `ApprovalOption` |
| | `acp.rs` | `AcpSession`, `SessionRecord`, `AcpUsage`, `Tokens`, `EmitParams`, `ModelParams` |
| | `machine.rs` | `ExecParams`, `ExecResult`, `FileParams`, `StatsParams`, `Sample`, `LinkState`, `Link`, `MachineStats`, `REPLY_LIMIT` |
| | `screen.rs` | `WatchParams`, `ScreenUpdate`, `Line`, `Run`, `Color`, `TerminalResizeParams`, `TerminalParams` |
| | `handover.rs` | `HandoverCheckParams`, `HandoverParams`, `Handover` |
| shed-server | `lib.rs` | `Error`, `Paths`, the modules, `relay` |
| | `serve.rs` | `serve`, `listen`, `bind`, `accept`, `stop_signals`, `raise_open_files`, `sample`, `record_stats`, `watch`, `kill` |
| | `dispatch.rs` | `Session`, `connection`, `respond`, `invalid`, the gates, `undocumented`, the included handler table |
| | `call.rs` | `Call`, `Reply`, `json`, `empty`, `io_error`, `blocking` |
| | `files.rs` | `file`, `lines`, `within_limit` |
| | `config/` | `mod.rs` (`var`, `var_os`), `terminal.rs`, `sessions.rs`, `update.rs` |
| | `registry/mod.rs` | `Registry`, `Entry`, lookups (`entry`, `live`, `get`, `ls`, `named`, `fresh`, `files`, `rules`) |
| | `registry/save.rs` | `StateFile`, `Saved`, `Saver`, `answering`, `keep_saved`, `write_state`, `close` |
| | `registry/load.rs` | `readable`, `load`, `adopt`, `end_left_over`, `history`, `prune` |
| | `registry/handover.rs` | `Frozen`, `Handed`, `inherit`, `picked_up`, `freeze`, `thaw`, `bridges` |
| | `registry/run.rs` | `run`, `spawn`, `size`, `valid_name` |
| | `registry/warm.rs` | `recipe`, `take`, `spares`, `warm_up`, `stale_spares` |
| | `registry/exit.rs` | `refresh`, `exited`, `to_wait`, `mark_kill`, `killed`, `rm`, `release_lanes`, `running` |
| | `registry/family.rs` | `children`, `busy_children`, `orphans`, `deliver`, `note`, `working` |
| | `registry/guard.rs` | `Guard`, `largest`, `process_name`, `sample`, `record`, `machine_stats`, `counts` |
| | `registry/io.rs` | `read`, `read_screen`, `send`, `typed`, `keys`, `resize`, `state`, `to_reported` |
| | `registry/stream.rs` | `Stream`, `pump`, `subscribe`, `attach` |
| | `registry/acp.rs` | `emit`, `model`, `bridge` |
| shed | `main.rs` | `Cli`, `main` |
| | `cx.rs` | `Cx`, `On`, `Server`, `connect`, `start_server`, `serve_detached`, `links` |
| | `table.rs` | `table`, `columns`, `state_word`, `totals` |
| | `out.rs` | the only `print!` site, plus `print` and `raw` |
| | `service.rs` | `Job`, `jobs`, `install`, `uninstall`, `start_job`, `written` |
| shed-core | `terminal/mod.rs` | `Terminal`, `Detached`, `spawn`, `spawn_with`, `adopt`, `start`, `detach`, `master`, `release`, `Drop` |
| | `terminal/status.rs` | `Status`, `Rule`, `Rules`, `Wake`, `report`, `set_report`, `records`, `set_rules`, `screen_rule`, `wait_until`, `set_idle_after` |
| | `terminal/io.rs` | `write`, `press`, `resize`, `subscribe`, `read_loop`, `feed_loop`, `write_loop`, `stage`, `batches` |
| | `terminal/exit.rs` | `reap`, `wait_pid`, `try_wait`, `usage`, `kill`, `runtime` |
| | `screen/mod.rs` | `Size`, `Color`, `Pen`, `Run`, `Snapshot`, `Screen` |
| | `screen/grid.rs` | `Grid`, `Cell`, `Glyph`, the cursor and scroll moves |
| | `screen/vt.rs` | `impl Perform for Grid`: the escape sequences |
| | `screen/paint.rs` | `sgr`, `paint`, `line_text`, `write_line`, `runs`, `join` |
| | `screen/report.rs` | `Report`, `STATES`, `FOREIGN`, `program_status`, `text64`, `PROBE_REPLY` |

Unit tests move with the code they test. Every public path stays: `shed_proto::RunParams`,
`shed_core::terminal::Terminal`, `shed_core::screen::Report`, `shed_server::config::warm_max`.
