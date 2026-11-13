# ADR: cool the hot files by domain, not by layer

## Summary

Decision: the three append-only docs (changelog, protocol doc, OpenRPC file) become fragment-fed generated files that drover regenerates instead of merging; the protocol prose gets one source, the API types' own docs; then the four per-layer monoliths (proto types, server entry, registry, CLI entry) are split into per-domain modules behind unchanged public paths.
Slices: (1) changelog fragments, generated marks, CI base ref; (2) protocol doc rendered from the type docs, its test split by domain; (3) shed-proto by domain; (4) shed-server: serve loop, handlers, client, registry by concern; (5) the CLI by domain.
Open questions: should slices 3 to 5 be three follow-up issues, one per crate, queued refactor and serial (recommended), or built in this PR after slices 1 and 2?

## Question

Fourteen days, 59 merged PRs: five files needed 7 to 10 update rounds each (`shed-server/src/lib.rs` 10,
`registry.rs` 9, `docs/protocol.md` 9, `protocol_doc.rs` 9, `shed/src/main.rs` 7) and drover predicted a
conflict on `docs/protocol.md` 139 times. What in the layout makes unrelated PRs collide, and what change
stops it without a rewrite?

## What the history shows

Per merged PR, which hot files it touched (see the README for the table). Two patterns:

- **Append-only files every PR touches.** `CHANGELOG.md` (28 PRs, 28 predictions) is a hand-kept list
  everyone appends to. `docs/protocol.md` and `docs/shed.openrpc.json` are generated, so any two PRs that
  change a type both regenerate them and the outputs interleave; git cannot merge them and a lane re-blesses
  by hand each update round. The doc test also carries the protocol's prose paragraphs as Rust strings, so a
  wording change edits the test and the markdown together.
- **One file per layer.** Every feature runs the same spine: a type in `shed-proto/src/lib.rs`, an example in
  `protocol_doc.rs`, an arm in `shed-server/src/lib.rs`'s dispatch, a method on `registry.rs`, a subcommand
  in `shed/src/main.rs`. The ACP, machines, screen and state work of the fortnight was independent, yet it
  landed in the same five files. `lib.rs` and `registry.rs` co-changed in 24 of 59 PRs, `main.rs` and
  `lib.rs` in 23, `main.rs` and `protocol_doc.rs` in 21.

`acp.rs` (16 PRs, 1 update round) and `shed-core/src/terminal.rs` (5 rounds, one domain: the PTY) are hot
because one domain is busy, not because domains collide. They stay as they are.

## Options

- **A. Accept it.** About one update round per merged PR; each costs a rev round and a CI run. Rejected: the
  queue is the bottleneck and the cost grows with every PR in flight.
- **B. Marks only.** Mark the changelog and the two protocol docs with drover's `generated` (#890) and feed
  the changelog from one fragment file per PR. Cheap (no code moves), removes the three worst conflict
  sources and the update rounds on the docs, but leaves the four Rust monoliths, where the real rounds are.
- **C. Marks, one source for the prose, then split the monoliths by domain with stable paths.** The proto
  crate keeps `shed_proto::RunParams` and friends as re-exports, the registry stays one type with its
  `impl` spread over files by concern, the server keeps one dispatch match whose arms are one line each,
  the CLI keeps one `Command` enum. Every move is mechanical and reviewable with `git diff --color-moved`.
- **D. Feature folders across layers** (`features/acp/{proto,server,cli}.rs`). Fights the crate split and
  the `may_use` layer rule in `.herdsman/arch.toml`; needs a workspace reshuffle. Rejected.
- **E. Generate the CLI from the protocol.** The CLI arms are small and hand-shaped (tables, exit codes);
  the churn is not there. Rejected.

## Decision

Option C, in five slices in this order; slices 1 and 2 first because they cost no source lines and remove
the three biggest conflict sources on their own.

1. **Changelog fragments and generated marks.** `changelog.d/<kind>-<issue>.md`, one file per PR; a script
   renders `CHANGELOG.md`'s Unreleased section from them and folds them into a release section on
   `--release`. `.herdsman/arch.toml` gains `generated = [CHANGELOG.md, docs/protocol.md,
   docs/shed.openrpc.json]` and `generate = "just generate"`; CI's rules job checks out two commits and runs
   `drover arch-check --base HEAD^1`. A frozen or generated file leaves `drover hotspots`, and an update round
   reruns `generate` instead of hand-merging.
2. **One source for the protocol prose.** The paragraphs the doc test writes about a terminal, the events and
   the screen updates move to the `///` docs on `TerminalInfo`, `Event` and `ScreenUpdate`; schemars already
   carries field and type docs into `openrpc()`, so the test renders `docs/protocol.md` from the OpenRPC
   document plus its golden examples. The golden examples stay typed Rust (a renamed field fails to compile)
   but split by domain under `tests/protocol_doc/`.
3. **shed-proto by domain.** `lib.rs` keeps the envelope (`Request`, `Response`, `RpcError`, `ErrorCode`,
   `Hello`, `Method`, `openrpc`); `terminal.rs`, `events.rs`, `acp.rs`, `screen.rs`, `machine.rs` hold the
   params and results, each re-exported at the root so no caller changes.
4. **shed-server by concern.** `serve.rs` (listener, connection, `Session`, dispatch), `terminal.rs` (kill,
   read, wait, watch), `files.rs` (exec, file), `client.rs` (`Client`, `notification`, `relay`); `lib.rs`
   keeps `Error`, `Paths` and the module list. `registry.rs` becomes `registry/` with `mod.rs` (the type,
   lookups), `save.rs` (state file, saver), `lifecycle.rs` (spawn, adopt, run, exit, kill, rm, prune),
   `family.rs` (parent and child), `input.rs` (send, keys, resize, attach, state), `acp.rs` (bridge, emit,
   model).
5. **The CLI by domain.** `main.rs` keeps `Cli`, `Command`, `main` and the `run` match; `cmd/terminal.rs`,
   `cmd/acp.rs`, `cmd/machine.rs` hold the arg structs and handlers; `client.rs` holds the `Server` wrapper
   and `connect`; the json-or-text `print` helper joins `out.rs`.

Slices 3 to 5 are pure moves: the tests do not change, `cargo test` output is identical, and the diff under
`--color-moved=dimmed-zebra` shows only moved blocks plus `use`, `mod` and `pub(crate)` lines. They should be
three issues queued `--refactor --serial` so each lands in one review between other work; this PR carries
slices 1 and 2 (the test changes shape there, so this PR is not `--refactor`).

## Consequences

- Adding a field to an ACP session touches the ACP files of each crate and no file a machines PR touches.
  Adding a method still touches four registration points (`Method`, `schemas()`, `dispatch`, `Command`), one
  line each, grouped by domain so concurrent additions land in different hunks.
- `docs/protocol.md` keeps its shape and its prose, now the same text `shed schema` serves; a wording change
  is one `///` edit in the type that owns it, and `just generate` writes both docs and the changelog.
- Every lane runs `just generate` before pushing (part of `just check`); CI fails a stale generated file.
- The changelog's Unreleased section is still readable at HEAD; released sections are hand-kept history the
  script leaves alone. `git log -- CHANGELOG.md` no longer tells who added what; `changelog.d` and the PR do.
- Source lines: slice 1 adds none, slice 2 about +30 (prose into `///`), slices 3 to 5 about +150 of `use`
  and `mod` headers together, against 233 of headroom under the 10,000 cap today. The last slice may need the
  standard cap ask with `drover cap`'s queue-wide view; nothing else in this plan grows the code.
- `git blame` and `drover why` cross the moves with `-M`/`--follow`; each move commit's why note names the
  slice. Deeper nesting under `registry/` and `cmd/` is the price of one file per concern.
- The generated check compiles `shed-proto` in CI's rules job (one extra `cargo test` of a small crate).
