# Hotspots, week of 2026-11-09: one file per domain, not one per layer

## Summary

Decision: the three append-only docs (changelog, protocol doc, OpenRPC file) become fragment-fed generated files that drover regenerates instead of merging; the protocol prose gets one source, the API types' own docs; then the four per-layer monoliths (proto types, server entry, registry, CLI entry) are split into per-domain modules behind unchanged public paths.
Slices: (1) changelog fragments, generated marks, CI base ref; (2) protocol doc rendered from the type docs, its test split by domain; (3) shed-proto by domain; (4) shed-server: serve loop, handlers, client, registry by concern; (5) the CLI by domain.
Open questions: should slices 3 to 5 be three follow-up issues, one per crate, queued refactor and serial (recommended), or built in this PR after slices 1 and 2?

The decision and its options are in [adr.md](adr.md). This file has the evidence, how the code reads
afterwards, the shapes, the boundaries and the slices.

## The evidence

Of 59 PRs merged to staging since 2026-10-31, the ones that touched three or more hot files (`x` = touched):

| PR | changelog | server lib | cli main | registry | protocol.md | doc test | proto lib | openrpc | acp.rs |
|---|---|---|---|---|---|---|---|---|---|
| #119 ACP lanes survive restart | . | x | x | x | . | . | . | . | x |
| #113 bounded buffers, heartbeats | . | x | x | x | x | x | x | x | x |
| #96 full OSC 7501 | . | x | x | x | x | x | x | x | x |
| #115 protocol mismatch, install | x | x | x | . | x | x | x | x | . |
| #107 usage_update, sessions | . | x | x | x | x | x | x | x | x |
| #110 pastes whole, unsent line | . | x | x | x | x | x | x | x | . |
| #92 parent and child terminals | x | x | x | x | x | x | x | x | . |
| #95 session options to the agent | . | . | x | x | x | x | x | x | x |
| #105 exec and file without a terminal | . | x | x | . | x | x | x | x | . |
| #103 shed schema as OpenRPC | . | x | x | . | x | x | x | x | . |
| #99 wide characters, queries | . | . | x | x | x | x | x | . | . |
| #58 hold one SSH link per machine | x | x | x | x | x | x | x | . | . |
| #53 structured read, preview | x | . | x | x | x | x | x | . | x |

Independent work (ACP, machines, screen, state, serve) lands in the same five Rust files because each file
is a layer, not a domain. The two generated docs conflict whenever two PRs change any type. The changelog
conflicts whenever two PRs exist. Files hot for one busy domain (`acp.rs`, `shed-core/src/terminal.rs`,
`screen.rs`) and files with no update rounds (`ci.yml`, `Cargo.*`, `justfile`, the test files, which are
already one per domain) are left alone; `README.md`'s 22 PRs are its status line and command list, 6
predicted conflicts, not worth a mechanism.

## How it reads afterwards

A PR that adds `cost` to an ACP session's usage:

1. `crates/shed-proto/src/acp.rs`: the field, its `///` line. That line is the protocol doc.
2. `crates/shed-proto/tests/protocol_doc/acp.rs`: the golden `usage()` example gains the field (the compiler
   insists).
3. `crates/shed-server/src/registry/acp.rs` and `crates/shed/src/cmd/acp.rs`: the behaviour.
4. `changelog.d/added-123.md`: one bullet. `just generate` rewrites `CHANGELOG.md`, `docs/protocol.md` and
   `docs/shed.openrpc.json`; `just check` fails if that was skipped.

It touches nothing a machines PR touches. A PR that adds a method also edits the four registration points,
one line each: a `Method` variant and its `schemas()` arm (`shed-proto/src/lib.rs`), a `dispatch` arm
(`shed-server/src/serve.rs`), a `Command` variant (`shed/src/main.rs`). Arms sit in domain groups, so two
PRs adding to different domains change different hunks.

A release: `scripts/changelog.py --release 0.1.0` moves the fragments into a dated section and deletes them;
`just generate` is then a no-op until the next fragment.

A reader of `docs/protocol.md` sees the same document: the same lead, one section per method with its
description and a request and response, then the events, the screen updates and the error table. The method
descriptions are `Method::summary`, the three prose sections are the `///` docs of `TerminalInfo`, `Event` and
`ScreenUpdate`, read back from the OpenRPC document's `components.schemas.<Type>.description`.

## Data shapes

A changelog fragment, `changelog.d/<kind>-<issue>[-<slug>].md`:

```
kind  = added | changed | fixed | removed | deprecated | security
issue = the GitHub issue number
body  = one or more markdown bullets, as they appear under `### <Kind>` today
```

The script (`scripts/changelog.py`, stdlib only, run with `-I`) replaces the text between `## [Unreleased]`
and the next `## [` with the fragments grouped by kind in that order and sorted by issue number; a file whose
name does not match the grammar fails it. With `--release <version>` it writes `## [<version>] - <date>`
instead and removes the fragments. Both are idempotent.

The arch config, `.herdsman/arch.toml`, top-level keys before the first table:

```toml
generated = ["CHANGELOG.md", "docs/protocol.md", "docs/shed.openrpc.json"]
generate = "just generate"
```

`just generate` runs `scripts/changelog.py` and `SHED_BLESS=1 cargo test -p shed-proto --features schema --test protocol_doc`;
`just check` runs `generate` before `arch`. CI's rules job checks out with `fetch-depth: 2` and `just arch`
passes `--base HEAD^1` on pull requests.

The protocol doc's render input is `shed_proto::openrpc()` as it is today; no new type. Each method's
section uses `methods[].name`, `methods[].description`, the golden pair; the three prose sections use the
component descriptions named above.

## Module boundaries

The layer rule stays: core and proto use nothing, server uses core and proto, cli uses server. Inside a
crate, after the moves:

| crate | file | holds | may call |
|---|---|---|---|
| shed-proto | `lib.rs` | envelope: `Request`, `Response`, `RpcError`, `ErrorCode`, `Hello`, `Method`, `openrpc`; `pub use` of every domain module | all domain modules |
| | `terminal.rs` | `RunParams`, `Parent`, `LsParams`, `Target`, `ReadParams`, `ReadResult`, `Logged`, `SendParams`, `KeysParams`, `ResizeParams`, `StateParams`, `Reported`, `WaitParams`, `Children`, `WaitOutcome`, `WaitResult`, `State`, `TerminalInfo`, `Resources`, `Policy` | `acp` (for `AcpSession` in `TerminalInfo`) |
| | `events.rs` | `EventsParams`, `Event`, `EventKind`, `ApprovalOption` | `terminal`, `acp` |
| | `acp.rs` | `AcpSession`, `AcpUsage`, `EmitParams`, `ModelParams` | nothing |
| | `screen.rs` | `WatchParams`, `ScreenUpdate`, `Line`, `Run`, `Color`, `TerminalResizeParams`, `TerminalParams` | nothing |
| | `machine.rs` | `ExecParams`, `ExecResult`, `FileParams`, `StatsParams`, `Sample`, `LinkState`, `Link`, `MachineStats`, `REPLY_LIMIT` | nothing |
| shed-server | `lib.rs` | `Error`, `Paths`, the module list, `pub use` as today | everything |
| | `serve.rs` | `serve`, `listen`, `accept`, `connection`, `Session`, `respond`, `dispatch`, `readonly`, `raise_open_files`, `stop_signals`, `HEARTBEAT` | `terminal`, `files`, `registry`, `links`, `screen` |
| | `terminal.rs` | `kill`, `rm`, `read`, `input`, `wait`, `wait_children`, `watch`, `sample`, `record_stats`, `machine_stats` | `registry`, `links` |
| | `files.rs` | `exec`, `file`, `within_limit` | `runner` only |
| | `client.rs` | `Client`, `notification`, `relay` | `proto` only |
| | `registry/mod.rs` | `Registry`, `Entry`, `Stream`, `load`, `refresh`, `entry`, `live`, `get`, `ls`, `dir`, `counts`, `subscribe`, `sample`, `record`, `machine_stats` | `events`, `stats`, `input` |
| | `registry/save.rs` | `StateFile`, `Saved`, `Saver`, `write_state`, `close`, `prune` | `mod` |
| | `registry/lifecycle.rs` | `spawn`, `adopt`, `end_left_over`, `run`, `exited`, `mark_kill`, `killed`, `rm`, `to_wait`, `release_lanes`, `running`, `valid_name` | `mod`, `family`, `acp` |
| | `registry/family.rs` | `children`, `busy_children`, `orphans`, `deliver`, `note`, `working` | `mod` |
| | `registry/input.rs` | `send`, `typed`, `keys`, `resize`, `attach`, `state`, `to_reported` | `mod` |
| | `registry/acp.rs` | `bridge`, `emit`, `model` | `mod`, `crate::acp` |
| shed | `main.rs` | `Cli`, `Command`, `main`, `run`, `other_build` | `cmd::*`, `client`, `service`, `attach`, `watch` |
| | `client.rs` | `Server`, `connect`, `serve_detached`, `links` | `shed_server` |
| | `cmd/terminal.rs` | `RunArgs`, `SendArgs`, `ParentArgs`, `WaitArgs`, `run_terminal`, `ls`, `read`, `send`, `state`, `wait`, `events`, `top`, `table`, the value parsers | `client`, `out` |
| | `cmd/acp.rs` | `AgentsCommand`, `to_acp`, `agents_cmd`, `bridge` | `client`, `out` |
| | `cmd/machine.rs` | `MachineCommand`, `ExecArgs`, `machine`, `machine_stats`, `Listed`, `exec`, `file`, `stdin_bytes`, `raw`, `relay` | `client`, `out` |
| | `out.rs` | the only `print!` site, plus `print(json, value, text)` | nothing |
| | `service.rs` | install, uninstall, doctor, `serve`, `setup` (as today plus the two moves) | `client` |

The registry's files are one `impl Registry` spread over a module tree; nothing outside `registry/` names a
submodule. `files.rs` is the only server handler with no registry; it keeps `Command::new` calls behind
`shed_core::runner` as the ban requires.

## Statechart: a changelog entry

```
                 PR adds changelog.d/<kind>-<n>.md
  (none) ───────────────────────────────────────▶ fragment
                                                    │  just generate (every lane, CI's arch-check)
                                                    ▼
                                           rendered under ## [Unreleased]
                                                    │  scripts/changelog.py --release X.Y.Z
                                                    ▼
                                           under ## [X.Y.Z] - date; fragment deleted
```

The generated docs have one state: equal to what `generate` writes. A PR that leaves them stale fails
`drover arch-check`, which restores the file; an update round reruns `generate` instead of merging.

## Slices

| # | what | files | src lines | mark |
|---|---|---|---|---|
| 1 | fragments, generated marks, CI base ref | `changelog.d/`, `scripts/changelog.py`, `justfile`, `.herdsman/arch.toml`, `ci.yml`, `CONTRIBUTING.md` | 0 | this PR |
| 2 | prose into `///`, doc test by domain | `shed-proto/src/lib.rs`, `tests/protocol_doc/{main,terminal,acp,machine,screen,render}.rs`, the two generated docs | about +30 | this PR |
| 3 | shed-proto by domain | `shed-proto/src/{lib,terminal,events,acp,screen,machine}.rs` | about +45 | follow-up, `--refactor --serial` |
| 4 | shed-server by concern | `shed-server/src/{lib,serve,terminal,files,client}.rs`, `registry/` | about +75 | follow-up, `--refactor --serial` |
| 5 | the CLI by domain | `shed/src/{main,client}.rs`, `cmd/`, `out.rs`, `service.rs` | about +35 | follow-up, `--refactor --serial` |

Order matters: 1 and 2 first, because they end the three biggest conflict sources at no source cost, and
because the generated marks make the later moves' doc churn free. Each of 3 to 5 is one mechanical move
reviewed with `git diff --color-moved=dimmed-zebra`; its PR body states that `cargo test --workspace` lists
the same tests passing before and after. The cap has 233 lines of headroom today; the last move may need
the standard `drover cap` ask, and nothing here adds behaviour.

Existing fixtures that break the arch check (`ci/fixtures/arch-*`) need no change; slice 1 adds one
fixture, a stale generated file, so `just arch` proves the new check bites, as the other gates do.
