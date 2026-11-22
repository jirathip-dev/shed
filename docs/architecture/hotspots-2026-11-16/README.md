# Hotspots, week of 2026-11-16: a row is a file (#246)

## Summary

Decision: most of the 36 files are hot because each holds a table that every feature adds a row to: API methods, error codes, server handlers, commands, doctor rows, tests, fake-agent prompts, doc sections. Each table becomes a folder with one file per row, and the build collects the folder. Three kinds of file are regenerated instead of merged (the dependency lock, the test goldens, the README command table). Large single-purpose files are split by concern. What cannot be split (package manifests, the justfile, drover's own two files) stays whole under a size limit.
Slices: (0) marks, the bench jobs and the ACP doc split, no code moved: 18 files leave the list; (1) API methods and error codes as files, API types by area; (2) server: one handler file per method, registry and settings by concern; (3) commands and doctor rows as files, README table generated; (4) tests by topic, fake-agent prompts as files, goldens regenerated; (5) terminal and screen by concern.
Open questions: (1) run slices 1 to 3 before the queued issues that add a method or a command (#209, #217, #218) start (recommended), or after them? (2) Lists come out in file-name order, so the methods in the protocol doc, the commands in the help text and the README table become alphabetical: accept (recommended), or number the files to keep today's order?

Status: proposed (architect lane, 2026-11-21), read on staging `5eb5a7b`. The decision and the `.drover/arch.toml`
lines: [adr.md](adr.md). File formats and generated code: [shapes.md](shapes.md). Who may call whom:
[boundaries.md](boundaries.md). #116's slices 3 to 5 (proto, server and CLI by domain) were never built; this
plan replaces them.

## How it reads afterwards

**A PR that adds a method, `code.grep`,** adds these files and edits no line another PR shares:

| file | holds |
|---|---|
| `crates/shed-proto/methods/code_grep.md` | wire name, params and result types, where it runs, then the prose |
| `crates/shed-proto/src/code.rs` | `GrepParams`, `GrepResult` (the area's own file) |
| `crates/shed-proto/tests/protocol_doc/code.rs` | its golden example (the area's own file) |
| `crates/shed-server/src/methods/code_grep.rs` | the handler, one `call` function |
| `crates/shed/src/cmd/code.rs` | the subcommand (the `code` command's own file) |
| `docs/commands/code.md` | its README row |
| `crates/shed/tests/cli/code.rs` | its tests |
| `changelog.d/added-208.md` | its changelog line |

`just generate` then rewrites `docs/protocol.md`, `docs/shed.openrpc.json`, `README.md` and `CHANGELOG.md`. There
is no enum variant to add, no `ALL` length to bump, no `name`, `summary` or `schemas` arm, no dispatch arm, no
`Command` variant and no entry in `links::remote` or `Server::new`. A method file with no handler, or a handler
with no method file, does not compile.

**A PR that adds a flag to `run`** edits `RunParams` in `shed-proto/src/terminal.rs`, one sentence on its own line
in `methods/run.md`, `registry/run.rs`, `cmd/run.rs`, `docs/commands/run.md` and a test under `tests/cli/`. Two
such PRs still meet in those files, on different lines: today both rewrite the same 1,800-character line of
`summary` and the same README row.

**Other rows:** an error code is `crates/shed-proto/errors/<name>.md`; a doctor row is
`crates/shed/src/doctor/<nn>_<row>.rs`; a server setting goes in its area's file under `shed-server/src/config/`;
a fake-agent prompt is `crates/shed/tests/fake_acp/<word>.py`; a test goes in its topic's file under
`crates/shed/tests/<area>/`; an arch-check fixture is a folder `ci/fixtures/arch-<case>/`; an ACP doc topic is
`docs/acp/<topic>.md`; a bench job is `.github/workflows/bench-<name>.yml`. None needs a line anywhere else.

## The 36 files

PRs and update rounds are the issue's (83 merged PRs, 14 days). "shared ≤ N" is drover's mark for a file many
PRs touch by nature, held under N lines.

| file | PRs / rounds | why it is hot | after | slice |
|---|---|---|---|---|
| `crates/shed/src/main.rs` | 37 / 17 | the command table, three per-command lists in `Server::new`, every handler | `src/cmd/<command>.rs`, collected; `main.rs` keeps the global flags and `main`, shared ≤ 80 | 3 |
| `crates/shed-proto/src/lib.rs` | 35 / 16 | five parallel lists per method (22 of its commits edited `summary`), four per error code, every type | `methods/*.md` and `errors/*.md`, collected; types by area; `lib.rs` keeps constants and modules, shared ≤ 80 | 1 |
| `crates/shed-server/src/lib.rs` | 34 / 19 | dispatch arms in four places, the serve loop, handlers | `src/methods/<method>.rs`, collected; `serve.rs`, `dispatch.rs`, `call.rs`, `files.rs`; `lib.rs` shared ≤ 200 | 2 |
| `crates/shed-server/src/registry.rs` | 34 / 17 | one `impl` of 51 functions for every concern | `registry/`, 11 files by concern; `mod.rs` shared ≤ 350 | 2 |
| `crates/shed/tests/acp.rs` | 20 / 4 | 36 tests in one file | `tests/acp/<topic>.rs` | 4 |
| `crates/shed/tests/cli.rs` | 18 / 12 | 29 tests in one file | `tests/cli/<topic>.rs` | 4 |
| `docs/acp.md` | 17 / 3 | 16 sections in one file | `docs/acp/<topic>.md`; the intro is `docs/acp/README.md`, frozen | 0 |
| `crates/shed-core/src/terminal.rs` | 15 / 12 | start, status, input and output, exit in one `impl` | `terminal/` by concern; `mod.rs` shared ≤ 350 | 5 |
| `justfile` | 13 / 1 | `arch` lists one line per fixture (10 of its PRs) | a loop over `ci/fixtures/arch-*/`; shared ≤ 135 | 0 |
| `README.md` | 12 / 7 | the command table, one long line per command (9 of its PRs) | rows in `docs/commands/*.md`; generated | 3 |
| `crates/shed/tests/server.rs` | 12 / 10 | 15 tests in one file | `tests/server/<topic>.rs` | 4 |
| `.github/workflows/ci.yml` | 11 / 0 | six jobs in one file | each PR-only bench job in its own file; shared ≤ 100 | 0 |
| `ci/cap.toml` | 11 / 0 | the cap; `ci/cap.d/` replaced it in #222 | frozen | 0 |
| `crates/shed-proto/tests/protocol_doc/acp.rs` | 11 / 1 | the ACP types' examples; split by area in #116, the area is busy | shared ≤ 300 | 0 |
| `crates/shed/tests/fake_acp.py` | 11 / 3 | a chain of 16 prompts | `tests/fake_acp/<word>.py`, loaded by name; the core shared ≤ 260 | 4 |
| `Cargo.toml` | 10 / 2 | pins; Cargo reads one file | shared ≤ 70 | 0 |
| `Cargo.lock` | 8 / 1 | written by Cargo | generated | 0 |
| `crates/shed-server/Cargo.toml` | 8 / 1 | one manifest | shared ≤ 55 | 0 |
| `crates/shed-server/src/config.rs` | 8 / 0 | one function per setting, added at the end | `config/` by area; `mod.rs` shared ≤ 40 | 2 |
| `crates/shed-core/src/screen.rs` | 7 / 7 | grid, escapes, painting and state reports in 1,260 lines | `screen/` by concern | 5 |
| `crates/shed/src/service.rs` | 7 / 4 | `doctor` is a list of rows; most of its PRs edit it | rows in `src/doctor/*.rs`, collected; the job code stays, shared ≤ 180 | 3 |
| `crates/shed/tests/golden/cli.txt` | 7 / 4 | written by a test | generated | 4 |
| `crates/shed-core/Cargo.toml` | 6 / 1 | one manifest | shared ≤ 30 | 0 |
| `crates/shed-core/src/runner.rs` | 6 / 2 | the one place a process starts; 175 lines | shared ≤ 200 | 0 |
| `crates/shed-core/tests/terminal.rs` | 6 / 2 | 158 lines of tests | shared ≤ 170; a new test is a new file, which Cargo collects | 0 |
| `crates/shed-proto/tests/protocol_doc/terminal.rs` | 6 / 0 | the terminal types' examples | shared ≤ 230 | 0 |
| `crates/shed/tests/machines.rs` | 6 / 6 | 10 tests in one file | `tests/machines/<topic>.rs` | 4 |
| `crates/shed/tests/reliability.rs` | 6 / 3 | 16 tests in one file | `tests/reliability/<topic>.rs` | 4 |
| `.drover/rules.md` | 5 / 1 | drover's file, 30 lines at most by its own rule | shared ≤ 30 | 0 |
| `crates/shed-core/src/lib.rs` | 5 / 4 | modules and `Error`, 37 lines | shared ≤ 45 | 0 |
| `crates/shed-server/src/links.rs` | 5 / 4 | one concern; its list of methods moves to the method files in slice 2 | shared ≤ 250 | 0 |
| `crates/shed-server/src/providers.rs` | 5 / 0 | one concern | shared ≤ 230 | 0 |
| `crates/shed-server/src/providers/claude.rs` | 5 / 1 | already one entry of a collected folder; the entry is busy | shared ≤ 340 | 0 |
| `crates/shed/tests/acp_bench.rs` | 5 / 2 | one bench harness | shared ≤ 340 | 0 |
| `crates/shed/tests/golden/acp.txt` | 5 / 1 | written by a test | generated | 4 |
| `crates/shed/tests/service.rs` | 5 / 4 | 9 tests in one file | `tests/service/<topic>.rs` | 4 |

## Statecharts

A listed file:

```
            its slice merges
  listed ───────────────────────┬─▶ gone      (moved into a folder; the new files start at zero)
                                ├─▶ frozen    (an old list: a PR that adds a line fails, naming the folder)
                                ├─▶ generated (a conflict is settled by `just generate`, never by hand)
                                └─▶ shared    (under its limit)
  shared ── reaches its limit ──▶ the next feature goes in a new file; only an ADR raises a limit
  gone ──── a new file is listed again ──▶ it was cut along the wrong line: the next report says so
```

A row, from file to running code:

```
  file added to a collected folder
        │ cargo build (the folder changed, so the build script runs again)
        ▼
  read ── unknown key, no wire name, a code or name used twice ──▶ build fails, naming the file and the key
        │
        ▼
  table written to OUT_DIR ── a method with no handler, a handler with no method ──▶ compile error
        │
        ▼
  compiled in ── `just generate` ──▶ docs/protocol.md, docs/shed.openrpc.json, README.md
```

## Slices

| # | what | files leaving the list | queue marks |
|---|---|---|---|
| 0 | marks; `arch` fixtures as a loop; bench workflows; `docs/acp/`; `Cargo.lock` generated | 18 | plain |
| 1 | `shed-proto`: `methods/`, `errors/`, types by area | 1 | `--serial` |
| 2 | `shed-server`: `methods/`, `serve.rs`, `dispatch.rs`, `registry/`, `config/` | 3 | `--refactor --serial` |
| 3 | `shed`: `cmd/`, `doctor/`, `docs/commands/`, README table | 3 | `--serial` |
| 4 | tests by topic, `fake_acp/`, goldens generated | 9 | plain |
| 5 | `shed-core`: `terminal/`, `screen/` | 2 | `--refactor --serial` |

Slice 0 moves no Rust and can start now. Slices 1, 2 and 3 run in that order: 2 reads the per-method facts slice 1
puts in the method files, and 3 is easier once the server names its handlers. Slices 4 and 5 depend on nothing.
Each move is checked with `git diff --color-moved=dimmed-zebra` and `cargo test --workspace` listing the same
tests before and after. Slices 1 and 3 are not `--refactor`: the order of the generated docs and of `--help`
changes, and `Method` gains three functions.

Source lines (the cap counts `crates/*/src`, 21,960 of 23,000 today), estimated: slice 1 about -330 (the tables
and the prose leave `src`), slice 2 about +285, slice 3 about +145, slice 5 about +60: about +160 in all, each
new file's `use` lines and function header.
