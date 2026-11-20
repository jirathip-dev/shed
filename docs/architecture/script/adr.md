# ADR: scripts in a JavaScript subset, on shed's own machine, behind a guard (#192)

## Summary

Decision: shed gets a `script` method. A caller sends one program in a JavaScript subset and the server runs it beside the terminals, in a new pure crate that shed builds itself (lexer, parser, compiler, a small virtual machine; no third-party engine, no new dependency). The program calls shed's methods through the same path a socket request takes; a call that acts is first shown to a guard command named in the server's config file, and is refused when there is none. Steps, memory, calls, logs and a deadline bound every run, and a failed run still returns its logs, with the error's line and column.
Slices: (1) parser, +1,700; (2) values and heap, +510; (3) compiler and virtual machine, +1,650; (4) built-ins, +950; (5) promises and the host interface, +400; (6) the server's `script` method and the guard, +460; (7) the command, `--on`, cancelling across machines, the skill doc, +160. About +5,830 lines on 13,215: one cap raise from 13,250 to 19,500 before slice 1.
Open questions: (1) the subset costs about 5,800 lines where a JSON plan costs about 1,400: accept (recommended)? (2) should reading a file from a script need the guard too (recommended: yes)? (3) a caller can only say who it is, shed cannot prove it, so the guard may use it to refuse more, never to allow more: accept (recommended)? (4) one cap raise to 19,500 up front (recommended), or one per slice?

**Status:** proposed, for the owner. Read on staging `ce4424e`.
Usage with the worked examples, the grammar, the value model, the shapes, the boundaries, the statecharts,
the errors, the guard's tables, the limits, the tests and the slices: [README.md](README.md).

## The question

A client that drives many terminals pays one round trip per step: drover opens a connection and sends
`hello` for every call, a remote call is a whole `shed connect` over SSH, and a lane pays one Bash call per
step and reads every result in full. The server answers requests one per line but cannot run steps that
depend on each other, or filter, inside one trip. So: what does a caller send so that N dependent calls and
a filter are one trip, which a model writes correctly the first time, which cannot hurt the server that
holds every terminal, and which cannot do what the caller's own guarded Bash call could not?

Decided by the owner before this ADR: no third-party engine or search library; the engine is shed's own
safe Rust in its own crate; the line cap does not shape the design; each call that acts runs a guard first;
no guard means those calls are refused.

## Options

Lines are non-test Rust under `crates/*/src`. Every option also needs the same 620 lines outside the
engine (the wire shapes, the server's method, the guard, the CLI); the last column includes them.

| | option | engine | in all | verdict |
|---|---|---|---|---|
| a | **A JavaScript subset, compiled to instructions for a small machine** | 5,210 | 5,830 | chosen |
| a′ | The same subset, walked as a tree by async Rust | about 4,500 | 5,100 | 700 lines cheaper, and rejected for them: see "Inside the choice" |
| b | A JSON plan: steps, references to earlier results, map and filter with simple predicates | about 750 | 1,370 | can express the three worked examples and little past them |
| c1 | A small language of shed's own | about 3,300 | 3,900 | every caller learns it from the skill doc |
| c2 | A Rust-syntax subset, interpreted | about 5,700 | 6,300 | the largest, and Rust without its compiler |
| c3 | A Python subset (added here) | about 5,400 | 6,000 | no gain over (a) |
| | Compiled Rust | | | rejected |
| | An embedded engine (QuickJS, Boa, Rhai, Lua) | | | ruled out by the owner |

- **(b) the plan** is the honest small option. Its 750 lines are a path language for references (200),
  predicates (150), the map, filter, pick and line operations (200), limits and errors (100) and the driver
  (100). It does the three examples. It does not branch on a result, retry, catch one lane's failure and go
  on, or reshape text, and each of those is a new operation with its own syntax: a language grown by
  accident, in JSON. A model has seen none of it; a wrong path is `null`, not an error with a line. Its
  natural user is drover, which generates text; the owner's first users are lanes, which write it.
- **(c1) an own language** saves a third of (a) by dropping destructuring, template strings and promises.
  The saving is paid in the thing this issue exists to remove: a lane's mistake in a language it met ten
  lines ago is one more round trip.
- **(c2) the Rust-syntax subset** has to lex and parse what it then ignores (types, generics, lifetimes,
  `&`), needs `match`, `?`, `Option`, iterator chains and the `format!` family before a model's Rust runs,
  and handles shed's JSON as `v["name"].as_str()`. Without the type and borrow checks a program that runs
  here need not be Rust and Rust need not run here, while a model writes Rust leaning on the compiler's
  errors, which are not there.
- **Compiled Rust**: it needs `rustc` on every host, compiling takes seconds per script, and native code
  cannot be held to fuel or memory limits or forced through the guard.
- **(a)** is what models already write without being taught: the skill doc is shed's method list, not a
  language manual. Both reference designs chose it for that reason (Claude Code's Code Mode; jcode's
  `codemode`). Its cost is size, in a crate that touches nothing.

## Inside the choice

- **A machine, not a tree walk (a over a′).** An interpreter that suspends at `await` is either async Rust
  recursing through the tree, or instructions with frames kept as data. The first puts the script's call
  depth on the server's native stack, where an overflow is an abort that takes every terminal with it, and
  keeps live values inside Rust futures where nothing can list them. With frames as data, depth is a
  counted limit, fuel is one count per instruction, the roots for the memory count are a list, a run is
  plain data that is `Send` and is cancelled by dropping it, and the engine is tested with no runtime.
- **Memory is what is live.** Counting allocations would refuse a loop that reads 2,000 screens and keeps
  ten lines. Reference counts leak every closure that can see itself. An arena with a mark and sweep, in
  safe Rust, counts exactly what a script still holds.
- **Built-ins never call back.** `map`, `filter`, `sort`, `then` and the like are written in the subset
  (`prelude.js`, 120 lines) and compiled with each program. Fuel, depth and `await` work inside a callback
  with no special case, and the machine never recurses natively at run time.
- **A script's failure is a result, not an RPC error.** `RpcError` carries a code and a message. As
  `exec` answers with an exit code, `script` answers `{error, logs, calls, ms}`: the logs, which are how a
  lane debugs in one more trip, survive every failure, the deadline included.
- **Refuse rather than guess.** No implicit conversion, `==` only against `null`, no `Math.random` or
  `Date` (the engine has no entropy and no clock). What is left out fails with a message that says what to
  write. The three differences that raise no error (`sort`'s default order, strings as code points,
  objects in template strings) are listed in the README and go in the skill doc.
- **Where the guard is set.** Not in the request: a caller would name `true`. Not the environment of
  `shed serve`: changing it means editing the service and restarting. The config file's `[script]` table,
  read when a run starts, as `machine.links` reads `[machines]` again.
- **What the guard is given** is shed's own JSON (method, params, machine, caller), not an agent's hook
  format: shed names no agent outside `providers/`. drover adapts its lane hook to it.
- **What cancels a run.** Not the end of input: drover's remote call runs `shed connect` with the request
  on stdin, which stops writing at once and then reads the reply. The signal is the peer closing its whole
  socket, which the server can tell apart (below) and asks for once a second while a script runs.
- **A call starts when called, and a run that returns waits for its calls.** As in JavaScript, and so that
  a forgotten `await` on `shed.send` still sends. A run that fails drops its calls and names them.
- **`Pattern` moves to shed-proto.** The engine may use proto only and scripts match with shed's pattern.
  Its one user today is the server's `wait`, and its syntax is already part of `wait`'s contract.

## Decision

1. **The language** is the JavaScript subset of the README's tables: a program is the body of an async
   function, and anything outside the subset is a `SyntaxError` naming the construct.
2. **The engine** is the new crate `shed-script`: lexer, parser, compiler, a machine with heap-kept
   frames, an arena heap, the built-ins. Pure: no socket, file, process, thread, clock, environment, async
   runtime or `unsafe`; its dependencies are `shed-proto` and `serde_json`.
3. **One way out**: `Host::call(method, params) -> future`. The engine knows no method name.
4. **The host is the server's `dispatch`**, with a session of its own per call, polled inside the script's
   request: validation, events, saving and the handover count are those of a socket request.
5. **Every method has a class** in shed-proto (`read`, `guarded`, `refused`), a `match` that does not
   compile without a new method's arm. `shed.methods()` and `shed.describe(name)` come from the OpenRPC
   document.
6. **The guard** runs before every call of a guarded method: a command from `[script]` in the server's
   config file, the call as JSON on its stdin, through `runner::exec` with a 10 s timeout. Exit 0 allows.
   Any other exit, a failure to start, a timeout or no guard at all is a `GuardError` thrown at the call.
7. **The limits** are counted in the engine (50,000,000 steps, 64 MiB live, 200 frames, 2,000 calls, 32 in
   flight, 64 KiB of logs) except the deadline (60 s, at most 600), which the server adds with tokio. Eight
   scripts run at once per server.
8. **The reply** is `{result, logs, calls, ms}` or `{error, logs, calls, ms}`, within `REPLY_LIMIT`; over
   it, it is an error that says to return less.
9. **The surface**: the `script` method (`code`, `timeout`, `machine`, `caller`); `shed script -f <file>`,
   `shed script -`, `shed --on <machine> script`; `skill/script/` for lanes.

## Consequences

- **Size.** shed's source grows by 44%. All but 620 lines are in one crate that the layer rule keeps away
  from everything else, that is deterministic (no clock: the same program and answers give the same steps
  everywhere) and that is tested alone.
- **The server stays safe by construction, not by care**: no native recursion at run time; the workspace
  lints already forbid `unwrap`, indexing, `as` and `panic!`; a script hands its thread back every 10,000
  steps; the worst case is 8 runs of 64 MiB.
- **The guard is a policy check, not a sandbox.** Guard, config file and server belong to one user, as an
  agent's own hook settings do. Text sent into a shell is a command, and the guard sees the text, not what
  the shell makes of it. Reads are not guarded: a script can read any terminal's screen and history, as
  `shed read` can.
- **A guarded call costs a process** (tens of milliseconds); reads cost none. A script of hundreds of
  guarded calls spends its deadline on the guard.
- **Fail closed is the default everywhere.** Until a host's config names a guard, scripts there read and
  cannot act. That already covers two of the three examples. Writing the `[script]` line on each host is a
  host change, the owner's or drover's.
- **Nothing is rolled back.** What a script did before an error, a limit, the deadline or a cancel stays
  done; a call dropped in flight may or may not have landed. The error names the methods in flight.
- **Cancelling is prompt on a local socket** (within a second) and over a held link (slice 7 ends the
  `ssh` it started for the script). Through a caller's own `ssh` it is as prompt as `ssh` reports the
  loss; the deadline bounds the rest.
- **A script longer than 10 s defers a handover.** It is a request in flight like any other, so a
  self-update pass that meets one reports busy and tries again on its next pass. Cutting it instead would
  leave the caller not knowing what was sent.
- **Not JavaScript in the corners.** Code that leans on conversion, `this`, classes, regexes, `switch` or
  `finally` fails with a message; strings count code points. The skill doc carries the list.
- **The protocol gains a method**, an addition: an older server answers `NO_SUCH_METHOD`, and
  `shed --on` says to update the shed there.
- **Later, each its own issue:** drover's guard command and generated scripts; arguments passed beside the
  code; `finally` and `switch` if lanes keep reaching for them.

## Checked by running

**The worked examples are valid and their replies exact.** [check.mjs](check.mjs) takes the four
programs from README.md, runs them in Node against a fixture host with a guard and a virtual clock, and
compares each reply with the one shown (Node 24.22.2):

```console
$ node docs/architecture/script/check.mjs | grep -E '^(ok|DIFF)'; echo $?
ok   blocked.js
ok   start.js
ok   errors.js
ok   refused.js
0
```

**A server can tell a peer that stopped writing from one that is gone.** drover's remote call
(`crates/core/src/shed.rs`, `exchange` and `connect`) passes the request as the stdin of `shed connect`,
whose relay then shuts down the writing side of its socket, so the end of input must not cancel.
[peer-closed.py](peer-closed.py) watches a Unix socket as tokio does (macOS 27.2, kqueue; its epoll branch
is for Linux and was not run here):

```console
$ python3 -I docs/architecture/script/peer-closed.py
connected:             ['write ready']
the peer wrote a line: ['read ready']
it stopped writing:    ['read closed']
still only that:       []
it closed its socket:  ['write closed', 'read closed']
```

`write closed` is what tokio reports as `Ready::is_write_closed()` on the write half. Slice 6's two tests
(`closing_the_connection_cancels_the_calls_in_flight`, `a_peer_that_only_stops_writing_gets_its_reply`) run
on Linux and macOS; where a platform did not report it, a run would end at its deadline instead.

## Open questions

1. **Size.** (a) is about 5,830 lines; (b), the JSON plan, about 1,370 and done in two slices.
   Recommended: (a). Lanes are the first users and the plan's limits show at the first `if`.
2. **Is `file.read` guarded?** It changes nothing, so by the letter it is a read. But it reads any file the
   server's user can, and a lane's own `cat` passes its hook. Recommended: guarded, so on a server with no
   guard a script reads terminals and nothing else.
3. **The caller is only what it says.** Over a Unix socket of one user, and through `ssh`, shed cannot
   prove which terminal sent a script; `shed script` sends `$SHED` and its folder, and the guard gets them.
   Recommended: accept, with the rule that a guard uses `caller` to refuse more (a reviewer's script may
   not act), never to allow more.
4. **The cap.** Recommended: one owner-approved raise to 19,500 before slice 1 (5,830 plus 455 of margin),
   as #175 did; else the per-slice totals in the README's table.

## `.drover/arch.toml`

Slice 1 adds the layer, lets the server use it, and adds the bans that make "pure" a check, with one
fixture (`ci/fixtures/arch-script-pure`) that breaks them; the bans on processes, sleeping, the clock and
`env::var` already cover every crate.

```toml
[[layer]]
name = "script"
paths = ["crates/shed-script/src/**"]
mods = ["shed_script"]
may_use = ["proto"]

[[layer]]
name = "server"
paths = ["crates/shed-server/src/**"]
mods = ["shed_server"]
may_use = ["core", "proto", "script"]
```

and one of these for each of `std::fs`, `std::net`, `std::os`, `std::io`, `std::thread`, `std::time`,
`std::env` and `tokio::`:

```toml
[[ban]]
pattern = "std::fs"
paths = ["crates/shed-script/src/**"]
except = []
reason = "the script engine is pure: it reaches the outside only through Host"
```

`scripts/dep-direction.sh` (`just deps`) gains one check: shed-script's dependencies are shed-proto and
serde_json, nothing else. Slice 7 adds the generated skill page, which `just generate` writes:

```toml
generated = ["CHANGELOG.md", "docs/protocol.md", "docs/shed.openrpc.json", "skill/script/methods.md"]
```

No central list grows. New code is new files (`shed-proto/src/script.rs` and `pattern.rs`,
`shed-server/src/script.rs` and `script/guard.rs`, `shed/src/script.rs`); the files many PRs edit get one
line each: a `Method` variant and its arms, one `dispatch` arm, one `Command` variant.
