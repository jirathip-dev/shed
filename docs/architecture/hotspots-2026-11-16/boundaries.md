# Boundaries: who may call whom

The crate layers in `.drover/arch.toml` stay as they are: core, proto and pattern use nothing; client and catalog
use proto; search uses pattern and proto; script uses proto and pattern; server uses all of those; the CLI uses
server and client. This plan adds no layer and moves no code across a crate. The rules below are inside a crate.

## The one new rule: a row never calls a row

A row file (a handler, a command, a doctor row, a fake-agent prompt) is called by its collected table and by
nothing else. Two rows that need the same code share it through a module outside the folder.

| folder | called only by | may call | may not | enforced by |
|---|---|---|---|---|
| `shed-server/src/methods/` | the table included in `dispatch.rs` | `call`, `registry`, `links`, `files`, `serve`, `handover`, `providers`, `stats`, `code`, `proto` | another handler | the handler modules are private to `dispatch`; `[[ban]]` on `super::` and `dispatch::` in the folder |
| `shed/src/cmd/` | the table included in `main.rs` | `cx`, `out`, `table`, `service`, `attach`, `watch`, `update`, `skill`, `doctor`, `shed_client`, `shed_server` | another command | `[[ban]]` on `super::` and `cmd::` in the folder |
| `shed/src/doctor/` | `cmd/doctor.rs`, through the included list | `service`, `update`, `shed_client`, `shed_server` | another row, `out` (a row returns `Row`s; `cmd/doctor.rs` prints) | `[[ban]]` on `super::` and `out::` in the folder |
| `shed-proto/methods/`, `errors/` | `shed-proto/build.rs` | they are data: a type a file names must exist in `shed_proto`, or the crate does not compile | | the compiler |
| `tests/fake_acp/` | `fake_acp.py` | the `agent` object it is handed (`update`, `send`, `wait_response`, `keep`) | importing another prompt | review; a prompt file is loaded by path, not on `sys.path` |

## Inside each crate

| crate | module | may call | never |
|---|---|---|---|
| shed-proto | `lib.rs` | every module (it only re-exports) | |
| | `envelope` | `method` (for `Method` in `Request::new`, `ErrorCode`) | an area module |
| | `method` | every area module, for the types the method files name | |
| | `terminal` | `acp` (`AcpSession` in `TerminalInfo`) | `events`, `machine`, `screen` |
| | `events` | `terminal`, `acp` | |
| | `acp`, `machine`, `screen`, `handover`, `code`, `input`, `agents` | nothing in the crate | each other |
| shed-server | `serve` | `dispatch`, `registry`, `links`, `stats`, `handover` | a handler |
| | `dispatch` | `call`, `links`, `handover`, the handlers | `registry` internals: it locks, the handlers act |
| | `call` | `registry` (the lock), `proto` | `dispatch`, a handler |
| | `files` | `shed_core::runner`, the file system | `registry` |
| | `registry/mod.rs` | its children, `events`, `stats`, `input`, `lane`, `providers` | `dispatch`, `serve`, a handler |
| | `registry/<concern>.rs` | `registry/mod.rs` and the crate's other modules | a sibling's private items: what two concerns share is in `mod.rs` |
| | `config/<area>.rs` | `config/mod.rs` (`var`, `var_os`) | `std::env` (the existing ban, now excepting only `config/mod.rs`) |
| shed | `main.rs` | `cx`, the command table | a handler's own helpers |
| | `cx` | `shed_client`, `shed_server::machines`, `service::start_job` | `cmd`, `out` |
| | `table`, `out` | nothing in the crate | |
| shed-core | `terminal/mod.rs` | its children, `screen`, `runner`, `clock`, `history`, `proctree` | |
| | `terminal/<concern>.rs` | `terminal/mod.rs`, `screen`, `runner`, `clock` | a sibling's private items |
| | `screen/vt.rs` | `screen/grid.rs`, `screen/report.rs` | `terminal` |
| | `screen/mod.rs`, `paint`, `report`, `grid` | each other, downward: `mod` → `paint` → `grid`; `report` uses none | `terminal`, `runner` |

The registry is still one type behind one lock. Its files are one `impl Registry` spread over child modules,
which see the private fields of `mod.rs`; nothing outside `registry/` names a child module. The same holds for
`Terminal` and `Screen`.

## Generated files and their one source

| file | written by | from | never edited by hand |
|---|---|---|---|
| `docs/protocol.md`, `docs/shed.openrpc.json` | the `protocol_doc` test under `SHED_BLESS` | the types, `methods/*.md`, `errors/*.md` | as today |
| `CHANGELOG.md` (Unreleased) | `scripts/changelog.py` | `changelog.d/` | as today |
| `README.md` (the command table) | `scripts/commands.py` | `docs/commands/*.md` | new; the rest of the README is hand-written and the script leaves it alone |
| `crates/shed/tests/golden/**` | the `golden::` tests under `SHED_BLESS` | the running binary | new |
| `Cargo.lock` | `cargo metadata` | the manifests | new |

All of them through `just generate`, which `drover arch-check` runs and compares.
