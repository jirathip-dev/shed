# ADR: hotspots week of 2026-11-16: a row is a file (#246)

## Summary

Decision: most of the 36 files are hot because each holds a table that every feature adds a row to: API methods, error codes, server handlers, commands, doctor rows, tests, fake-agent prompts, doc sections. Each table becomes a folder with one file per row, and the build collects the folder. Three kinds of file are regenerated instead of merged (the dependency lock, the test goldens, the README command table). Large single-purpose files are split by concern. What cannot be split (package manifests, the justfile, drover's own two files) stays whole under a size limit.
Slices: (0) marks, the bench jobs and the ACP doc split, no code moved: 18 files leave the list; (1) API methods and error codes as files, API types by area; (2) server: one handler file per method, registry and settings by concern; (3) commands and doctor rows as files, README table generated; (4) tests by topic, fake-agent prompts as files, goldens regenerated; (5) terminal and screen by concern.
Open questions: (1) run slices 1 to 3 before the queued issues that add a method or a command (#209, #217, #218) start (recommended), or after them? (2) Lists come out in file-name order, so the methods in the protocol doc, the commands in the help text and the README table become alphabetical: accept (recommended), or number the files to keep today's order?

Status: proposed (architect lane, 2026-11-21). [README.md](README.md) has the table of all 36 files and how a PR
reads afterwards; [shapes.md](shapes.md) the formats; [boundaries.md](boundaries.md) the call rules.

## Question

The report lists 36 files. Which of them are tables that every feature adds a row to, which are written by a
tool, which are one concern grown large, and which cannot be split at all? And what layout takes each one off
the list without the same collisions coming back under a new name?

## Where it stands

Counted on staging's first-parent history since 2026-11-07 (91 commits), by the function each hunk falls in.

- **A method is up to ten hand-kept lists.** Five in `shed-proto/src/lib.rs`: the `Method` enum, `ALL` and its
  length, `name`, `summary`, `schemas`. Up to five in `shed-server`: `readonly`, `linked`, two `match`es in
  `dispatch`, and `links::remote`. 22 of the 38 commits on the proto file edited `summary`, where each method's
  prose is one line (`run`'s is 1,800 characters).
- **A command is five more**: `Command`, the `run` match, and three lists in `Server::new` (`local`, `linkable`,
  `terminal`).
- **An error code is four lists** (`ErrorCode`, `ALL`, `code`, `name`), and each new one takes the next number.
- **The registry is one `impl` of 51 functions.** `run` was edited by 17 commits, `load` by 14, `Entry::new` by
  10, `read` by 9, `emit` by 8: five concerns, one file, 1,923 of the 2,000 lines a file may have.
- **`doctor` is a list of rows**: 10 of the 11 commits on `shed/src/service.rs` edited it.
- **The README's command table** took 9 of its 12 PRs; a row is one line.
- **`just arch`** names each fixture on its own line: 10 of the justfile's 13 PRs.
- **Tests**: `tests/acp.rs` is 36 tests in 1,971 lines; `cli.rs` took 12 update rounds, `server.rs` 10.
- **#116 decided** to split proto, server and CLI by domain (its slices 3 to 5). They were never built, and it
  kept the lists by hand. Since then the proto file has grown from 1,356 to 1,733 lines and the registry from
  1,275 to 1,923.
- **shed already collects folders**: `shed-catalog/build.rs` builds in `providers/*.toml`, `shed-server/build.rs`
  builds in `src/providers/*.rs`, `ci/cap.d/` and `changelog.d/` are one file per entry.
- **Queued work adds rows**: #208 and #209 a method and a subcommand each, #217 and #218 the `script` method
  and command. #238 edits `service.rs`.
- **arch-check**, at the build installed here (`48f08b4`), measures the size of any file a `[[size]]` rule names
  (the justfile, a workflow, `.drover/arch.toml` too), applies a tighter rule beside the general one, compares a
  `generated` entry that holds a `*` file by file, and lets a new file pass a `frozen` glob. Run on a scratch repo.

## Options

| | Option | Verdict |
|---|---|---|
| A | Mark all 36 `shared` under size limits, move nothing | no: the PRs leave drover's serial rule but still collide in the same lists |
| B | Build #116's slices 3 to 5 as written: files by domain, lists kept by hand, one line per method | no: a method still edits the same lists, a line in each, and `summary` stays one line per method |
| C | Rows as files in folders the build collects; other files split by concern, generated, or `shared` under a limit | **chosen** |
| D | Collect rows at link time with a registry crate | no: a new dependency, and a missing handler shows at run time, not at compile time |
| E | Derive every stem from the wire name by renaming four `Method` variants | no: cooee and drover name `Method::Input` and `Method::Model`; the stem is the variant instead |
| F | Tests as flat files, one binary per topic (Cargo's own collection) | only for small files: about 45 binaries instead of 8 would each link the server and run one after another, against a 180 s budget |

## Decision

1. **The rule.** A table that features add rows to is a folder with one file per row. A build script (or Cargo,
   or a loop) collects the folder; no file lists its rows. What two rows share lives outside the folder.
2. **Methods and error codes** (slice 1). `crates/shed-proto/methods/<stem>.md` holds a method's wire name, its
   params and result types, where it runs, and its prose. `errors/<stem>.md` holds an error's name, code and
   line. `shed-proto/build.rs` writes `Method` and `ErrorCode` from them. The per-method facts that sit in
   server lists today (`on`, `input`, `hello`) are keys of the method file and functions of `Method`.
3. **API types by area** (slice 1): `envelope`, `method`, `terminal`, `events`, `acp`, `machine`, `screen`,
   `handover`, beside today's `agents`, `code`, `input`. `lib.rs` re-exports each with `pub use <module>::*`.
4. **Handlers** (slice 2). `crates/shed-server/src/methods/<stem>.rs` is one `call` function.
   `shed-server/build.rs` writes the `match` from `Method` to it, with no `_` arm. `dispatch.rs` keeps the gates
   and reads the method's facts; `links::remote` and `readonly` lose their lists.
5. **Server by concern** (slice 2): `serve.rs`, `dispatch.rs`, `call.rs`, `files.rs`; `registry/` in eleven files
   under one type and one lock; `config/` by area.
6. **Commands and doctor rows** (slice 3). `crates/shed/src/cmd/<stem>.rs` holds a command's arguments, its
   `ON` and its `run`; `crates/shed/src/doctor/<nn>_<row>.rs` a row. A new `shed/build.rs` writes `Command`
   and the two tables. `Server::new` matches on no command.
7. **The README table** (slice 3) is generated from `docs/commands/<command>.md` by `scripts/commands.py`,
   between two markers; the rest of the README stays hand-written.
8. **Tests** (slice 4). Each of the six large files under `crates/shed/tests/` becomes `<area>/main.rs` (helpers
   and `mod` lines, still one binary) plus one file per topic. The fake agent's prompts become
   `tests/fake_acp/<word>.py`, loaded by name.
9. **Goldens and the lock are generated** (slices 4 and 0). Every test that writes a golden lives in its area's
   `golden.rs`; `just generate` runs those with `SHED_BLESS`, and `cargo metadata` for `Cargo.lock`.
10. **Core by concern** (slice 5): `terminal/` in four files, `screen/` in five.
11. **Docs and CI by area** (slice 0): `docs/acp/<topic>.md`; each pull-request bench job in
    `.github/workflows/bench-<name>.yml`; the `arch` recipe loops over `ci/fixtures/arch-*/`, and the nested
    `arch-script-pure/*` cases move up one level to be found by it.
12. **`shared` under a limit** for what is one concern or cannot be a folder: three manifests, the justfile,
    `ci.yml`, drover's `rules.md` and `arch.toml`, `runner.rs`, `links.rs`, `providers.rs`,
    `providers/claude.rs`, the two protocol example files, shed-core's `lib.rs` and its terminal test, the bench
    harness, and each module file a slice leaves behind. A limit is the file's size today plus about a tenth
    (`arch.toml` gets room for the lines below). When a file is full the next feature goes in a new file.
13. **Not done.** No variant is renamed. No new dependency. `RunParams`, `TerminalInfo` and `EventKind` keep
    their shape: they stay the busiest types, each in its area's file. `Cargo.toml` and the justfile are not
    split: Cargo and just each read one file by name.

## Consequences

- A feature that adds a method, a command, an error code, a doctor row, a prompt or a test adds files and edits
  no shared list. 18 files leave the list with slice 0, the other 18 over slices 1 to 5.
- The order of lists changes once: methods by wire name and errors by code in the two protocol docs, commands by
  name in `--help` and the README. `shed doctor` keeps its order through numbered file names.
- `Method` gains `route`, `input` and `needs_hello`. Nothing is removed and no wire name, type or field changes,
  so `PROTOCOL` stays and cooee and drover build unchanged.
- Two PRs that add the same row conflict as before (the same new file). Two that take the same error code no
  longer conflict in git: the merged tree fails to build, naming both files, at drover's premerge check or the
  next CI run.
- Four build scripts and their shared helper, about 220 lines (80 today), write the tables; the cap does not
  count them. The tables are in `OUT_DIR`, not in the repo: a compile error inside one points there, and names
  the row. The scripts read only files of this repo, so cooee's and drover's git dependency on `shed-proto`
  builds as before.
- A row file has no unit tests (`use super::*` is banned there); it is tested through the socket or the CLI.
- `just generate` also runs `cargo metadata`, `scripts/commands.py` and the golden tests, so `drover turn done`
  and CI's gate run those once more. Not timed here.
- Each open PR takes one update round per slice that moves a file it edits. Slice 3 waits for #238.
- The bench jobs keep their names, so required checks match as before. Each new workflow file carries its own
  concurrency group. `ci.yml` keeps `gate`, `stable` and `idle-terminals`, the only bench that runs on a push,
  so a release waits on the same jobs as today.
- Slice 2 rewords two lines of `.drover/rules.md` that name `shed-server/src/lib.rs` and `config.rs`.
- `git blame` and `drover why` follow a moved file with `-M`, a split one with `-C`.
- Source lines: about +160 over the five slices (README, Slices).

Run here, on a throwaway crate outside the repo with the workspace's clap 4.6.7 and schemars 1.2.3: a
`Subcommand` enum written by a build script from a folder parses plain, nested, optional and hidden
subcommands, the hidden one through `#[command(hide = true)]` on its `Args`; a method table written from
`key: value` files compiles, and an unknown key fails the build naming the file; `serde_json::Value` as a result
gives the schema `true`, as `rpc.discover` has today; `pub use <module>::*` passes pedantic clippy; clippy lints
the written code and rejects arms with the same body, so the writer merges them. Not run: any slice, and the
time the golden tests add to `just generate`.

## Open questions for the owner

1. **Order.** #209 (`code.find`), #217 and #218 (`script`) each add a method and a command. Run slices 1 to 3
   after #208 and #241 merge and before those three start (recommended): each then adds its rows as new files.
   Or after them: nothing waits, and each of them and every PR beside them pays the update rounds once more.
2. **Alphabetical lists.** File-name order makes the methods in `docs/protocol.md`, the commands in `shed --help`
   and the README table alphabetical. Accept (recommended): no file carries a number, and a reader finds a name
   by its letter. Or keep today's order with a two-digit prefix on every method and command file, as the doctor
   rows have.

## The `.drover/arch.toml` lines

Top-level keys go before the first table and replace today's `generated` line; tables go below the existing ones.
If a file lands over its limit, the slice's PR sets the limit to the landed size rounded up to the next ten and
says so in its body.

Slice 0:

```toml
frozen = ["ci/cap.toml", "docs/acp/README.md"]
generated = [
    "CHANGELOG.md",
    "docs/protocol.md",
    "docs/shed.openrpc.json",
    "crates/shed-script/tests/perf/counts.json",
    "Cargo.lock",
]
generate = "just generate"
shared = [
    ".drover/arch.toml",
    ".drover/rules.md",
    ".github/workflows/ci.yml",
    "Cargo.toml",
    "justfile",
    "crates/shed-core/Cargo.toml",
    "crates/shed-core/src/lib.rs",
    "crates/shed-core/src/runner.rs",
    "crates/shed-core/tests/terminal.rs",
    "crates/shed-proto/tests/protocol_doc/acp.rs",
    "crates/shed-proto/tests/protocol_doc/terminal.rs",
    "crates/shed-server/Cargo.toml",
    "crates/shed-server/src/links.rs",
    "crates/shed-server/src/providers.rs",
    "crates/shed-server/src/providers/claude.rs",
    "crates/shed/tests/acp_bench.rs",
]

[[collect]]
dir = "ci/cap.d"
glob = "*.toml"

[[collect]]
dir = "docs/acp"
glob = "**/*.md"

[[collect]]
dir = ".github/workflows"
glob = "bench-*.yml"

[[collect]]
dir = "crates/shed-core/tests"
glob = "*.rs"

[[collect]]
dir = "crates/shed-server/src/providers"
glob = "*.rs"

[[size]]
paths = [".drover/rules.md", "crates/shed-core/Cargo.toml"]
max_file_lines = 30

[[size]]
paths = ["crates/shed-core/src/lib.rs"]
max_file_lines = 45

[[size]]
paths = ["crates/shed-server/Cargo.toml"]
max_file_lines = 55

[[size]]
paths = ["Cargo.toml"]
max_file_lines = 70

[[size]]
paths = [".github/workflows/ci.yml"]
max_file_lines = 100

[[size]]
paths = ["justfile"]
max_file_lines = 135

[[size]]
paths = ["crates/shed-core/tests/terminal.rs"]
max_file_lines = 170

[[size]]
paths = ["crates/shed-core/src/runner.rs"]
max_file_lines = 200

[[size]]
paths = ["crates/shed-server/src/providers.rs", "crates/shed-proto/tests/protocol_doc/terminal.rs"]
max_file_lines = 230

[[size]]
paths = ["crates/shed-server/src/links.rs"]
max_file_lines = 250

[[size]]
paths = ["crates/shed-proto/tests/protocol_doc/acp.rs"]
max_file_lines = 300

[[size]]
paths = ["crates/shed-server/src/providers/claude.rs", "crates/shed/tests/acp_bench.rs"]
max_file_lines = 340

[[size]]
paths = [".drover/arch.toml"]
max_file_lines = 400
```

Slice 1, `shed-proto`:

```toml
shared = ["crates/shed-proto/src/lib.rs"]   # appended to the list above

[[collect]]
dir = "crates/shed-proto/methods"
glob = "*.md"

[[collect]]
dir = "crates/shed-proto/errors"
glob = "*.md"

[[size]]
paths = ["crates/shed-proto/src/lib.rs"]
max_file_lines = 80
```

Slice 2, `shed-server`. The first two `[[ban]]` tables replace the existing ones of the same pattern:

```toml
shared = [                                   # appended
    "crates/shed-server/src/lib.rs",
    "crates/shed-server/src/registry/mod.rs",
    "crates/shed-server/src/config/mod.rs",
]

[[collect]]
dir = "crates/shed-server/src/methods"
glob = "*.rs"

[[collect]]
dir = "crates/shed-server/src/registry"
glob = "*.rs"

[[collect]]
dir = "crates/shed-server/src/config"
glob = "*.rs"

[[size]]
paths = ["crates/shed-server/src/config/mod.rs"]
max_file_lines = 40

[[size]]
paths = ["crates/shed-server/src/lib.rs"]
max_file_lines = 200

[[size]]
paths = ["crates/shed-server/src/registry/mod.rs"]
max_file_lines = 350

[[ban]]
pattern = "Hello::ours"
paths = ["crates/*/src/**"]
except = [
    "crates/shed-proto/src/**",
    "crates/shed-client/src/**",
    "crates/shed-server/src/methods/hello.rs",
    "crates/shed-server/src/dispatch.rs",
]
reason = "shake hands through shed_client; only the server's hello handler answers one, and dispatch's own tests send one"

[[ban]]
pattern = "env::var"
paths = ["crates/*/src/**"]
except = ["crates/shed-server/src/config/mod.rs"]
reason = "read the environment through shed_server::config"

[[ban]]
pattern = "super::"
paths = ["crates/shed-server/src/methods/**"]
except = []
reason = "a handler shares code through a module outside methods/, never through another handler"

[[ban]]
pattern = "dispatch::"
paths = ["crates/shed-server/src/methods/**"]
except = []
reason = "a handler shares code through a module outside methods/, never through another handler"
```

Slice 3, the CLI:

```toml
generated = ["README.md"]                    # appended
shared = ["crates/shed/src/main.rs", "crates/shed/src/service.rs"]   # appended

[[collect]]
dir = "crates/shed/src/cmd"
glob = "*.rs"

[[collect]]
dir = "crates/shed/src/doctor"
glob = "*.rs"

[[collect]]
dir = "docs/commands"
glob = "*.md"

[[size]]
paths = ["crates/shed/src/main.rs"]
max_file_lines = 80

[[size]]
paths = ["crates/shed/src/service.rs"]
max_file_lines = 180

[[ban]]
pattern = "super::"
paths = ["crates/shed/src/cmd/**", "crates/shed/src/doctor/**"]
except = []
reason = "a command or a doctor row shares code through a module outside its folder, never through another row"

[[ban]]
pattern = "cmd::"
paths = ["crates/shed/src/cmd/**"]
except = []
reason = "a command shares code through a module outside cmd/, never through another command"

[[ban]]
pattern = "out::"
paths = ["crates/shed/src/doctor/**"]
except = []
reason = "a doctor row returns its rows; cmd/doctor.rs prints them"
```

Slice 4, the tests:

```toml
generated = ["crates/shed/tests/golden/**"]  # appended
shared = ["crates/shed/tests/*/main.rs", "crates/shed/tests/fake_acp.py"]   # appended

[[collect]]
dir = "crates/shed/tests"
glob = "*/*.rs"

[[collect]]
dir = "crates/shed/tests/fake_acp"
glob = "*.py"

[[size]]
paths = ["crates/shed/tests/fake_acp.py"]
max_file_lines = 260

[[size]]
paths = ["crates/shed/tests/*/main.rs"]
max_file_lines = 300
```

Slice 5, `shed-core`:

```toml
shared = ["crates/shed-core/src/terminal/mod.rs"]   # appended

[[collect]]
dir = "crates/shed-core/src/terminal"
glob = "*.rs"

[[collect]]
dir = "crates/shed-core/src/screen"
glob = "*.rs"

[[size]]
paths = ["crates/shed-core/src/terminal/mod.rs"]
max_file_lines = 350
```
