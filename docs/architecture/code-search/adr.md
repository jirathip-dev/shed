# ADR: shed's own code search, in two leaf crates (#193)

## Summary

Decision: shed gets its own code search in two small crates with no outside dependency. One is today's pattern matcher, moved out of the core and given groups, alternation and counts on an engine that cannot blow up on a hostile pattern. The other walks a tree by its ignore files, finds lines by their fixed text first, and outlines Rust, TypeScript/JavaScript, Python, Markdown and TOML by the shape of their lines. Grep answers are grouped under the function or type that holds each line, find ranks files with reasons, and six fixed limits are errors that name the limit. A measured prototype finds the same lines as rg on eleven queries, at single-threaded rg's speed whenever the pattern holds fixed text.
Slices: (1) the pattern crate, also behind wait-for-output, +380; (2) outlines and the outline method, +690; (3) the walk and grouped grep, +690; (4) find, +250. About +2,010 source lines in all.
Open questions: (1) the script design (#192) moves the matcher into the API-types crate, this one gives it a crate of its own that the script crate may then use: own crate (recommended), or the API-types crate? (2) wait-for-output takes the new syntax too, so brackets and the bar in a caller's existing pattern become operators: accept (recommended), or keep the old syntax there? (3) One cap raise of 2,200 before slice 1, on top of whatever #192 gets (recommended), or one per slice? (4) Let the file-read method take a line range, +15 lines in slice 2, so a caller on another machine can read just the span a result names: add (recommended), or leave it out?

**Status:** proposed, for the owner. Read on staging `ce4424e`. Usage, data shapes, module boundaries, the
statechart, the rules and the goldens: [README.md](README.md).

## The question

A lane's worktree is on the machine that runs it. Searching it from the orchestrator is `shed exec rg …`:
a process per query, on a machine that must have rg, answering `path:line:text` lines that say nothing
about where a hit sits. The next step is reading whole files. Can shed itself answer "where is this", "what
is in this file" and "which files matter", in a shape that names the twenty lines worth reading, with no
third-party search code, and fast enough that nobody goes back to `exec rg`?

## Options

| | option | verdict |
|---|---|---|
| A | Keep `exec rg` | a process per call, rg on every host, raw lines: the problem as filed |
| B | Run rg from the server and group its lines | still rg on every host, and the crate would start a process |
| C | The `ignore`, `regex` and `grep` crates, or tree-sitter | ruled out by the owner |
| D | In-house, with a lexer or parser per language | exact ranges, at several hundred lines a language; the line rules already agree with a bracket-matching lexer on 1,518 of the 1,519 symbols in shed's own Rust, and the odd one is the lexer's miss |
| E | **In-house: a pattern crate, a search crate, line rules** | chosen |

Inside E:

- **Where the matcher lives.** In `shed-core`, as today: the search crate would have to use core, which
  holds the PTY, tokio and `runner`, and #192's script crate, which may use `proto` only, could not reach
  it. Inside the search crate: `wait --output` and scripts would depend on search for a matcher. In
  `shed-proto`, as #192's ADR proposes for today's small matcher: it works for both, but this issue makes the
  matcher an engine of about 500 lines with its own tests, which is not an API type. Its own leaf crate:
  chosen; #192's `script` layer then reads `may_use = ["proto", "pattern"]`. Whichever of the two issues
  lands first makes the move, and the engine is the same code in either home (open question 1).
  `shed-core` loses 125 lines and still depends on no workspace crate.
- **The engine.** Today's matcher backtracks. With groups that is exponential: `(a+)+$` on a long run of
  `a` does not return, and the pattern comes from the caller. Chosen: compile to a small program and step
  every live state at once (Thompson's construction, a Pike VM without captures), so the time is at most
  pattern size times line length. A DFA would be faster on a pattern with no fixed text, for about 300
  more lines; not in v1.
- **Pattern or plain text by default.** Pattern, as rg and `wait --output` are. A query of plain
  characters is its own needle and never reaches the engine; `fixed` is for text full of operators. A
  construct shed lacks is an error that names it. Plain by default would answer `fn \w+_limit` with zero
  matches and no error.
- **Whose types.** The result is a wire contract, so it is defined once, in `shed-proto`, and the search
  crate returns it. Types of its own would add a conversion of about 120 lines.
- **The deadline.** The crate may not read a clock. The server passes a closure that does; a test passes
  one that turns true on its third call.
- **Living with fixed limits.** The error says where the search stopped and how to narrow; `count` answers
  with per-file counts and no lines, so narrowing is not a guess.
- **A matched line over the limit** fails the query and names the file; `--glob '!…'` leaves it out.
  Reporting the file as skipped would be a cut.
- **Nesting.** A function is a leaf: a match is grouped under its function, never under an `if` inside it.
- **The user's global ignore file** is not read. Finding it needs the environment, and an answer should
  depend on the tree alone.
- **One addition to the issue's list:** TOML tables in outlines, 10 lines, because every repo the fleet
  works on is configured in TOML.

## Decision

1. **`shed-pattern`**, a leaf crate on std alone: `crates/shed-core/src/pattern.rs` moves there and gains
   groups, alternation, counts, `\b` and anchors anywhere on the new engine, plus `needles()`, the fixed
   text every match must hold. `wait --output` uses it; its one import changes.
2. **`shed-search`**, a leaf crate on `shed-pattern` and `shed-proto`'s types: `grep`, `find` and `outline`
   take the params and return the results. It reads files in one module, starts no process, reads no
   clock and no environment, writes nothing and keeps nothing between calls.
3. **Walking.** A `.gitignore` subset: the files of the root, of every folder walked and of the folders
   above the root up to the repository's top, plus `info/exclude` (found through a worktree's `.git`
   file); the whole rule syntax, negation included. Hidden names are skipped unless asked for; `.git`
   always; symlinks are never followed; a NUL byte in the first 8 KiB makes a file binary. Entries are
   visited in name order, so an answer and a limit error repeat exactly.
4. **Matching.** Fixed text first: each file is scanned as bytes for the query's needles (Horspool, any
   ASCII case), and the engine runs only on the lines that hold one. A pattern with no needle runs on
   every line.
5. **Outlines.** One header table per language and one shared rule for the range, by indentation and
   closing brackets. A symbol has a kind, a name, a first and last line and a depth. Unknown extensions
   get blocks by indentation.
6. **Output.** Grep: files, each with groups, each the chain of symbols around its lines, outermost first,
   every symbol with its range. Find: the top files (10, at most 50) of a stated total, each with a
   score, short reasons and the symbols that earned them. Outline: the symbols in file order. Plain text
   from one renderer in the search crate; `--json` is the result as the socket sent it.
7. **Limits.** Files read 20,000, result rows 250, symbols 2,000, bytes read 256 MiB, a matched line
   4,096 bytes, 10 s. Over any of them the answer is the new error `LIMIT`, naming the limit, its value
   and where the search stood, and nothing else.
8. **Served** as `code.grep`, `code.find` and `code.outline`, each with `machine`, routed like `exec` and
   run on a blocking thread. A script (#192) calls them through `dispatch`; they read any file the
   server's user can, so there they take the class `file.read` gets (#192's open question 2).

## Benchmark

`bench/bench.py` with `bench/prototype.rs`, on shed at `ce4424e` (243 files searched, 1.6 MB of text) against
ripgrep 15.2.0. Apple M6, 12 cores, 32 GB, macOS 27.2, a shared build host at load average 43 during the
run. Each cell is the median of 15 runs of the whole process, wall time and then CPU time in brackets;
the CPU time is the steadier one on this host. `same` means both tools matched the same path and line
pairs.

| query | lines / files | rg | rg -j1 | prototype | rg bytes | shed text | shed JSON | same |
|---|---|---|---|---|---|---|---|---|
| `Pattern::new` -F | 3 / 2 | 10.3 (17.8) ms | 10.3 (8.3) ms | 9.8 (7.8) ms | 295 | 419 | 829 | yes |
| `spawn_blocking` -F | 9 / 4 | 8.9 (16.3) ms | 7.3 (6.4) ms | 6.9 (6.2) ms | 2,260 | 2,215 | 2,686 | yes |
| `wait` -F -w -g '*.rs' -g '!tests/' | 41 / 13 | 8.3 (8.1) ms | 7.4 (6.0) ms | 6.5 (5.3) ms | 4,083 | 4,369 | 7,460 | yes |
| `handover_refused` -F -i | 8 / 4 | 8.9 (16.4) ms | 8.0 (7.1) ms | 7.3 (6.5) ms | 1,339 | 1,622 | 2,468 | yes |
| `fn \w+_limit` | 11 / 7 | 11.5 (20.1) ms | 12.4 (10.1) ms | 8.9 (7.7) ms | 1,223 | 2,113 | 3,529 | yes |
| `TODO\|FIXME\|HACK` | 4 / 2 | 8.9 (17.1) ms | 8.3 (7.3) ms | 9.1 (8.3) ms | 435 | 497 | 769 | yes |
| `Method::(Exec\|FileRead\|FileWrite)\b` | 9 / 4 | 10.6 (18.7) ms | 11.2 (9.9) ms | 9.6 (8.5) ms | 820 | 860 | 1,506 | yes |
| `\d{2,}\.\d+\.\d+` -g '*.toml' | 0 / 0 | 9.8 (7.8) ms | 8.5 (7.2) ms | 4.9 (3.8) ms | 0 | 40 | 39 | yes |
| `^\s*pub (async )?fn [a-z_]+\(&mut self` | 6 / 2 | 9.3 (15.8) ms | 8.5 (7.5) ms | 7.8 (6.9) ms | 519 | 688 | 1,509 | yes |
| `[0-9a-f]{40}`, no needle | 156 / 1 | 9.7 (19.7) ms | 11.5 (10.5) ms | 26.0 (24.0) ms | 14,544 | 13,675 | 15,603 | yes |
| `zzzz_not_here` -F | 0 / 0 | 10.6 (18.0) ms | 10.7 (8.7) ms | 9.6 (7.7) ms | 0 | 41 | 40 | yes |

| other | rg or `cat` | prototype | their bytes | shed text | shed JSON |
|---|---|---|---|---|---|
| `self` -F, over the limit | | `LIMIT matches: over 250 at crates/shed-core/src/history.rs` | | | |
| the same with `--count` (`rg -c`) | 8.2 (20.0) ms | 7.7 (6.8) ms | 2,350 | 2,467 | 5,710 |
| outline `registry.rs`, 1,976 lines (`cat`) | 1.7 (1.1) ms | 2.1 (1.3) ms | 74,305 | 3,493 | 7,961 |
| outline `docs/protocol.md` (`cat`) | 2.2 (1.3) ms | 2.2 (1.3) ms | 28,173 | 1,341 | 2,742 |
| find `handover` (`rg -l -i`) | 9.9 (18.9) ms | 10.9 (9.0) ms | 683 | 1,447 | 2,187 |
| find `line cap` (`rg -l -i 'line\|cap'`) | 9.4 (23.2) ms | 11.3 (10.3) ms | 4,376 | 1,679 | 2,640 |
| find `wait output pattern` | 9.4 (22.4) ms | 13.6 (11.3) ms | 2,991 | 1,866 | 3,049 |

A tree far over the limits, to see where they bite: the local cargo registry's sources, 76,885 files and
1.6 GB, one run each.

| query | rg | rg -j1 | prototype, limits off | prototype, as designed |
|---|---|---|---|---|
| `spawn_blocking` -F | 1.1 (3.0) s | 1.8 (1.6) s | 2.4 (2.2) s | `LIMIT bytes` after 0.6 s |
| `fn poll_` -F | 0.5 (4.4) s | 2.2 (2.0) s | 2.6 (2.4) s | `LIMIT matches` after 0.3 s |
| `[0-9a-f]{40}`, no needle | 0.6 (5.4) s | 3.4 (3.3) s | 18.0 (14.9) s | `LIMIT matches` after 1.3 s |

What the numbers say:

- On shed's repo every query costs what starting a process costs, 5 to 14 ms, and the prototype is level
  with single-threaded rg. In the server there is no process to start.
- With a needle the prototype reads 1.6 GB at about 700 MB/s on one thread: 1.2 to 1.4 times `rg -j1`'s
  time and 2 to 5 times threaded rg's. The byte and file limits stop such a tree in under a second.
- With no needle the engine runs on every line: 2.5 times rg here, and 4.5 times `rg -j1`'s CPU time on the big tree, about 110 MB/s. This is
  the price of no regex crate. The byte limit and the deadline bound it.
- The text is about the size of `rg -n`'s. The saving is in what is read next: an outline is 21 times
  smaller than its file, and a group names a span of a few lines in place of the file.
- On the big tree the prototype counts 9 more files than rg for one pattern (4,775 against 4,766): all 9
  are under a folder named `tags`, which this host's global ignore file lists and the subset does not read.

## Out of scope for v1, and what a later issue needs

- **`trace`** (agentgrep: follow a name to its definition, callers and tests, ranked as regions). v1 has
  its two inputs: `grep -w name` gives every mention with its enclosing symbol, and the outline gives the
  definition. A later issue adds a relation per hit (definition, call, import, test) as one classifier
  function per language file, a ranker over regions that reuses find's scores, a rule for when a region is
  small enough to return whole, and a `code.trace` method with its own result. No parser. About +400 lines.
- **The seen-context contract** (the caller says what it has read, repeats come back compressed). It needs
  a content hash on each `Symbol` (FNV-1a, in-house, 10 lines), a `seen` list of path, range and hash in
  the params, and the rule that a group whose span is in `seen` comes back with its scope and no lines.
  The caller keeps the list; shed stays stateless. `Symbol` can gain the optional hash without breaking a
  client. About +120 lines.
- Smaller, each its own issue if wanted: following symlinks (needs loop detection by device and inode);
  the global ignore file (the server reads its path in `config.rs` and passes it in); a threaded walk (the
  gap to threaded rg above; needs an ordered merge to keep answers repeatable); context lines; a DFA for
  patterns with no needle; more languages, one file each.

## Consequences

- Any machine with shed answers a grouped search: no rg to install, no process per query. A query holds
  one blocking thread for at most 10 s and one file in memory at a time.
- `wait --output` gets the new syntax. `(`, `)` and `|` were plain characters there, and `^` and `$` were
  plain except first and last; a caller's pattern that holds them changes meaning, and an unbalanced `(`
  is now refused. Slice 1 lists drover's and cooee's patterns in its PR body (open question 2). #192's
  `text.matches(pattern)` gets the same syntax.
- A pattern with no fixed text is slower than rg by the factor above, and rg with threads is faster on
  very large trees. Both are bounded by the limits; neither shows on a lane's worktree.
- An answer depends on the tree and the query alone: no global ignore file, no configuration, name order.
- The outline is a set of line rules and can be wrong: a header-looking line in a multi-line string, items
  a macro writes. A wrong range puts a matched line under the wrong heading; it never drops the line,
  because matching does not depend on the outline.
- A long line in one tracked file (minified code, a JSON-lines fixture) fails every query that matches it
  until the caller excludes the file. That is the rule as decided; the error names the file.
- A new language is one file under `lang/`, one match arm and its goldens. Until then its files get blocks
  by indentation.
- The protocol grows by three methods and one error code. An older server on a link answers
  `NO_SUCH_METHOD`.
- `crates/shed-proto/src/lib.rs` is at 1,940 of the 2,000-line file limit; the registrations take it to
  about 1,975. The split in [hotspots-by-domain](../hotspots-by-domain/adr.md) slice 3 has to land before
  much else is added there.

## Slices and the cap

| | slice | `src/` | cap needed |
|---|---|---|---|
| 1 | `shed-pattern`; `wait --output` on it; `shed-core` loses its matcher | +380 | 13,628 |
| 2 | `shed-search` with outlines; the proto types; `code.outline`; `LIMIT` | +690 | 14,318 |
| 3 | the walk, the ignore rules, matching, grouped `code.grep`; the benchmark in `docs/bench.md` | +690 | 15,008 |
| 4 | `code.find` | +250 | 15,258 |

Staging is at 13,248 of 13,250 (`48d5312`), so slice 1 cannot land without a raise. Recommended: one
orchestrator-filed raise of 2,200 before slice 1 (2,010 plus a tenth), to 15,450 if nothing else moves,
summed with the rest of the queue by `drover cap`. #192 asks for 19,500 on its own count; with both it is
21,700. The alternative is four raises to the column above. Each is recorded in `docs/cap-history/`.
Slices 2 to 4 each rerun `just generate` for the two protocol documents.

## Hot files

No file is converted here, so nothing is frozen or collected. The feature is laid out so it adds none:

- One new file per crate for the domain (`shed-proto/src/code.rs`, `shed-server/src/code.rs`,
  `shed/src/code.rs`, `shed-proto/tests/protocol_doc/code.rs`). The per-layer files get registration lines
  only: about 35 in `shed-proto/src/lib.rs` over three slices, 2 in `shed-server/src/lib.rs`, 1 in
  `links.rs`, 4 in `shed/src/main.rs`.
- Languages are one file each under `crates/shed-search/src/lang/`; goldens are one set of files per case
  under `crates/shed-search/tests/golden/`, collected by a glob in the test.
- `docs/protocol.md` and `docs/shed.openrpc.json` are already `generated`.

## `.drover/arch.toml`

Slice 1 adds the `pattern` layer and its ban, and `pattern` to the server's `may_use`; slice 2 adds the
rest. Each ban comes with a fixture under `ci/fixtures/` that `just arch` proves it bites on, and the layer
line in `.drover/rules.md` is updated to match.

```toml
[[layer]]
name = "pattern"
paths = ["crates/shed-pattern/src/**"]
mods = ["shed_pattern"]
may_use = []

[[layer]]
name = "search"
paths = ["crates/shed-search/src/**"]
mods = ["shed_search"]
may_use = ["pattern", "proto"]

[[layer]]
name = "server"
paths = ["crates/shed-server/src/**"]
mods = ["shed_server"]
may_use = ["core", "proto", "pattern", "search"]

[[ban]]
pattern = "fs::"
paths = ["crates/shed-search/src/**", "crates/shed-pattern/src/**"]
except = ["crates/shed-search/src/walk.rs"]
reason = "shed-search reads files only in walk.rs; shed-pattern reads none"

[[ban]]
pattern = "std::net"
paths = ["crates/shed-search/src/**", "crates/shed-pattern/src/**"]
reason = "search and patterns open no socket"
```

The `server` block replaces the one there today; `cli` stays `may_use = ["server"]` and reaches the text
renderer through `shed_server::search`. The bans already in the file cover the new crates as they are:
`Command::new`, `std::process`, `thread::sleep`, `Instant::now`, `SystemTime::now` and `env::var` apply to
`crates/*/src/**`.
