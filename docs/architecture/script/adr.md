# ADR: scripts in a JavaScript subset, on shed's own machine, behind a guard (#192)

## Summary

Decision: shed gets a `script` method. A caller sends one program in a JavaScript subset and the server runs it beside the terminals, in a new pure crate that shed builds itself (lexer, parser, compiler, a small virtual machine; no third-party engine, no new dependency). The program calls shed's methods through the same path a socket request takes. A call that acts is first shown to a guard: one process for the whole run, named in the server's config file, which answers each call in well under a millisecond; with no guard named the call is refused. Steps, memory, calls, logs and a deadline bound every run. Speed has six targets, each with its measurement, and a gate: CI compares fixed scripts' step counts and peak memory with a committed baseline, and the reviewer of slices 3 to 7 reports before and after times.
Slices: (1) parser, +1,700; (2) values and heap, +550; (3) compiler and virtual machine, +1,680; (4) built-ins, +950; (5) promises and the host interface, +400; (6) the server's `script` method, the guard and the reply's list of acting calls, +650; (7) the command with `--check`, `--on`, cancelling across machines, the skill doc and the numbers from a real remote host, +170. About +6,100 lines, 270 more than the first version (the guard as one process, the collector taken in slices, the counters the gate reads, the acting list, `--check` and roles). Conformance is a vendored test262 subset and Node-made goldens, both tests. With the other queued work that is about 20,085 of the 20,300 in PR #205.
Open questions: none. The first three are decided as recommended and the cap by PR #205.

**Status:** proposed, for the owner; revised after the owner's note of 2026-11-20 (q2649: the guard as
one process, the Performance section, the gate; the open questions closed). Read on staging `ce4424e`,
lines counted again on `48d5312`.
Usage with the worked examples, the grammar, the value model, the shapes, the boundaries, the statecharts,
the errors, the guard's tables, the limits, the measuring, the tests and the slices: [README.md](README.md).

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

Lines are non-test Rust under `crates/*/src`. Every option also needs the same 820 lines outside the
engine (the wire shapes, the server's method and loop, the guard, the CLI); the last column includes them.

| | option | engine | in all | verdict |
|---|---|---|---|---|
| a | **A JavaScript subset, compiled to instructions for a small machine** | 5,280 | 6,100 | chosen |
| a′ | The same subset, walked as a tree by async Rust | about 4,550 | 5,370 | 700 lines cheaper, and rejected for them: see "Inside the choice" |
| b | A JSON plan: steps, references to earlier results, map and filter with simple predicates | about 750 | 1,570 | can express the three worked examples and little past them |
| c1 | A small language of shed's own | about 3,300 | 4,120 | every caller learns it from the skill doc |
| c2 | A Rust-syntax subset, interpreted | about 5,700 | 6,520 | the largest, and Rust without its compiler |
| c3 | A Python subset (added here) | about 5,400 | 6,220 | no gain over (a) |
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
- **A collection is a state of the run, not a pause of the server.** Marking 64 MiB of small cells in one
  go could hold a server thread for tens of milliseconds. The mark stack and the sweep position are data,
  so a collection is taken in the same 10,000-unit slices as steps, with the thread handed back between
  them. The script's own code waits meanwhile, so there is no write barrier and nothing concurrent.
- **Handing the thread back is the server's, and scripts never hold every worker.** The engine has no
  runtime, and a task that only wakes itself is polled again before tokio looks at its sockets: with 8
  such tasks on 4 workers another client's read took 18.6 ms at p99 (measured below). So the loop is the
  server's: a slice, then `yield_now`, which lets the reactor and the other clients go first (0.6 to
  2.9 ms). A built-in is not sliced (a `split` of 8 MiB is one hold), so a run steps only while it holds
  a permit, and there is one fewer than the worker threads: with 50 ms holds in every script the read
  stays at 0.1 to 0.5 ms, against 98 ms without the permits.
- **Built-ins never call back.** `map`, `filter`, `sort`, `then` and the like are written in the subset
  (`prelude.js`, 120 lines), compiled once per process. Fuel, depth and `await` work inside a callback
  with no special case, and the machine never recurses natively at run time.
- **A script's failure is a result, not an RPC error.** `RpcError` carries a code and a message. As
  `exec` answers with an exit code, `script` answers `{error, logs, calls, acting, ms}`: the logs, which are how a
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
- **One guard process for a run, not one for a call.** A process a call is tens of milliseconds (46 ms for
  a Python hook here), so a script of a few hundred guarded calls would spend its deadline in the guard.
  The guard is started at the run's first guarded call and answers one line for each call's line (0.02 ms
  measured). It is per run, not per server: a config change takes effect at the next run and no guard
  outlives the work it judged. One call is asked at a time and the answer carries the call's id, so a
  guard that printed a stray line or lost its place fails closed instead of allowing the wrong call.
- **A process a call stays, as a setting, never as a silent fallback.** A guard that is a plain hook (read
  the call, exit 0 or not) is set `guard_per = "call"`. The server never falls back by itself: a per-run
  guard that fails refuses, and each kind under the other's setting refuses too.
- **What cancels a run.** Not the end of input: drover's remote call runs `shed connect` with the request
  on stdin, which stops writing at once and then reads the reply. The signal is the peer closing its whole
  socket, which the server can tell apart (below) and asks for once a second while a script runs.
- **A call starts when called, and a run that returns waits for its calls.** As in JavaScript, and so that
  a forgotten `await` on `shed.send` still sends. A run that fails drops its calls and names them.
- **Patterns come from `shed-pattern` (#193).** Scripts match with shed's own pattern, the one `wait`
  uses, and #193 makes it a crate of its own; the engine uses that crate and not shed-proto, so its errors
  and its host's are its own types and the server maps them to the wire. Nothing moves to shed-proto.
- **The reply lists what acted.** Every call of a guarded method is in the reply's `acting` list with its
  params and its result or refusal, so a caller learns what a failed or stopped run had already done
  without a second trip.
- **Conformance has two sources, neither needing Node in CI.** A vendored subset of test262, picked by
  feature tags, run by a Rust harness; and goldens for the worked examples, made by Node (`check.mjs`)
  only when an example changes.

## Decision

1. **The language** is the JavaScript subset of the README's tables: a program is the body of an async
   function, and anything outside the subset is a `SyntaxError` naming the construct.
2. **The engine** is the new crate `shed-script`: lexer, parser, compiler, a machine with heap-kept
   frames, an arena heap, the built-ins. Pure: no socket, file, process, thread, clock, environment, async
   runtime or `unsafe`; its dependencies are `shed-pattern` (#193) and `serde_json`, not shed-proto.
3. **One way out**: `Host::call(method, params) -> future`. The engine knows no method name.
4. **The host is the server's `dispatch`**, with a session of its own per call, polled inside the script's
   request: validation, events, saving and the handover count are those of a socket request.
5. **Every method has a class** in shed-proto (`read`, `guarded`, `refused`), a `match` that does not
   compile without a new method's arm. `shed.methods()` and `shed.describe(name)` come from the OpenRPC
   document.
6. **The guard** is asked before every call of a guarded method: a command from `[script]` in the
   server's config file, started once per run through `runner::held`, given each call as one JSON line and
   answering one line with the call's id, within 10 s. Only `"allow":true` allows. A no, a guard that
   cannot start, ends, answers out of turn or takes too long, or no guard at all is a `GuardError` thrown
   at the call. `guard_per = "call"` is the fallback for a plain hook: a process per call through
   `runner::exec`, its exit status the answer.
7. **The limits** are counted in the engine (50,000,000 steps, 64 MiB live, 200 frames, 2,000 calls, 32 in
   flight, 64 KiB of logs) except the deadline (60 s, at most 600), which the server adds with tokio. Eight
   scripts run at once per server, and the worker threads less one step at once.
8. **The reply** is `{result, logs, calls, acting, ms}` or `{error, logs, calls, acting, ms}`, within
   `REPLY_LIMIT`; `acting` is every guarded call made, `{method, params, result}` or `{method, params,
   error}`. Over the limit it is an error that says to return less, never a cut result.
9. **The surface**: the `script` method (`code`, `timeout`, `machine`, `caller` with its `role`, `check`);
   `shed script -f <file>`, `shed script -`, `shed script --check` (parse and compile, no run, no call, no
   guard), `shed --on <machine> script`; `skill/script/` for lanes. The guard may refuse by
   `caller.role`; a role never allows more.
10. **Speed is specified and gated**: the six targets and the three-level gate of "Performance" below.
11. **Conformance** is test262's files for the subset's feature tags, vendored and run by a Rust harness in
    CI, plus Node-made goldens for the worked examples (README.md, "Conformance").

## Consequences

- **Size.** shed's source grows by 46%. All but 820 lines are in one crate that the layer rule keeps away
  from everything else, that is deterministic (no clock: the same program and answers give the same steps
  everywhere) and that is tested alone.
- **The server stays safe by construction, not by care**: no native recursion at run time; the workspace
  lints already forbid `unwrap`, indexing, `as` and `panic!`; a run hands its thread back every 10,000
  steps, collecting included, and one worker is always free of script steps; the worst case is 8 runs of
  64 MiB.
  On a host with one core there is one worker, and only the slices protect the other clients.
- **The guard is a policy check, not a sandbox.** Guard, config file and server belong to one user, as an
  agent's own hook settings do. Text sent into a shell is a command, and the guard sees the text, not what
  the shell makes of it. Reads are not guarded: a script can read any terminal's screen and history, as
  `shed read` can.
- **A run that acts costs one process**, started at its first guarded call (the guard's own start: 40 ms
  for Python here, less for a compiled one), then well under a millisecond a call; reads cost none. Only a
  `guard_per = "call"` guard still costs a process a call.
- **A guard is a loop now, not a hook.** It must answer every line with that line's id and keep nothing on
  stdout but answers. drover's guard command is written to that; an existing hook is used with
  `guard_per = "call"`, slowly.
- **An engine change that moves a count shows in the PR.** `counts.json` is generated and CI fails when it
  is stale, so the cost of every fixed script is reviewed like code. The times are a reviewer's, on a fleet
  host: CI never fails on a clock.
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

## Performance

What makes it fast is above: one round trip, one guard process, plain instructions, and a collector and a
loop that never hold a server thread. What keeps it fast is below. Times are for a release build on a
fleet host; the report prints the host's cores beside them.

| | what | target | measured by | first given by |
|---|---|---|---|---|
| 1 | The blocked-lanes example on a real remote host, 8 blocked lanes | 1 SSH session where today's per-call path opens 9. Wall time at most 1.5 times one remote `ls`, and at most a quarter of the per-call path's | `scripts/script-remote.sh <machine>`: both ways 10 times, the sessions counted by an `ssh` shim | slice 7's PR |
| 2 | The guard | under 1 ms added to a guarded call once the guard is up | the server's report: 2,000 `shed.state` calls through a guard that parses and answers each line, the whole run under 2 s; 200 of them with a process a call beside it | slice 6; the model below gives 0.02 ms against 46 ms |
| 3 | Engine speed | at least 25 million steps a second, so the whole fuel is at most 2 s of one core and a slice at most 0.4 ms | `spin.js` and `shapes.js`: their exact steps over the wall time | slices 3 and 4 |
| 4 | Parse and compile of a 10 KB script | at most 2 ms, so at most 50 ms at the 256 KiB limit | `ten-kb.js` through `Program::parse` 1,000 times, the median | slice 3 |
| 5 | The worst collector pause | for the server, none: no slice over 1 ms, with or without a collection. For the script itself: a 60 MiB heap of small cells collected in at most 150 ms, start to end | `hold.js`: the longest `step`, the longest collection; the largest collection's work is also a count in CI | slice 3 |
| 6 | Another client's `read` while 8 scripts run | p99 at most 3 ms, the idle p99 of the same run beside it | the server's report: 8 connections spinning to their fuel, a ninth doing 2,000 reads of a 24 by 80 screen | slice 6; the model below gives 0.6 to 1.0 ms |

Targets 3 to 5 are set before the engine exists, from what a machine of this shape in safe Rust should do;
slice 3's `baseline:` commit gives the first real numbers. A missed target is a FAIL for its slice and a
question for the owner, never a number a lane moves.

The gate has three levels; the files and commands are in README.md, "Measuring and the gate".

- **Exact, in CI.** The engine reads no clock, so a fixed script's steps, peak bytes and collections are
  the same on every machine. They are in a generated `counts.json`: CI fails when it is stale, and a
  regenerated one shows every count that moved as a line of the PR's diff. No timing, so no noise.
- **A ceiling, in CI.** Each fixed script carries a hand-written ceiling on its steps and peak bytes, its
  first counts plus 10%. Regenerating cannot raise it; a person does, with the reason in the PR body.
- **Times, in review.** Slices 3 to 7 are queued `--perf`: the first commit is the baseline, and rev's
  `Perf:` block gives before and after by `just script-bench` with the range of 5 runs. Slice 7's PR gives
  the end-to-end numbers from a real remote host.

## Checked by running

**The worked examples are valid and their replies exact.** check.mjs (since slice 5
[make.mjs](../../../crates/shed-script/tests/golden/make.mjs), which writes the goldens) takes the four
programs from README.md, runs them in Node against a fixture host with a guard (one process for the run:
20 ms to start, then nothing on its whole-millisecond clock) and a virtual clock, and compares each reply
with the one shown (Node 24.22.2):

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

**One guard process answers a call in a fiftieth of a millisecond; a process a call costs 46 ms.**
[guard-trip.py](guard-trip.py) asks a guard that parses each call and answers with its id, both ways
(macOS 27.2, Python 3.9.6, 12 cores, the host busy with other lanes at a load of 12 to 30):

```console
$ python3 -I docs/architecture/script/guard-trip.py
a process for each call            median   47.010 ms   p99   58.207 ms   (100 calls)
one process for the run: its start          42.965 ms   (with its first answer)
one process for the run: a call    median    0.019 ms   p99    0.040 ms   (1999 calls)
```

**Another client's read stays fast only when the server yields, and stays fast under long holds only with
the permits.** [pacing/](pacing/main.rs) is tokio 1.54.1 with the server's 4 workers: 8 tasks stand in for
scripts, and a thread outside the runtime sends 1,500 lines to an echo task, as another client's `read`:

```console
$ cargo run --release --manifest-path docs/architecture/script/pacing/Cargo.toml
4 workers, 1500 reads by another client, 1 ms apart
no script                                                  p50   0.014 ms   p99   0.126 ms   max   1.961 ms
8 scripts, 0.4 ms slices, wake itself                      p50   4.130 ms   p99  18.586 ms   max  22.788 ms
8 scripts, 0.4 ms slices, yield_now                        p50   0.231 ms   p99   0.643 ms   max   1.718 ms
8 scripts, 0.4 ms slices, yield_now, 3 step at once        p50   0.013 ms   p99   0.631 ms   max   4.545 ms
8 scripts, 50 ms holds, yield_now                          p50  47.919 ms   p99  98.501 ms   max 148.520 ms
8 scripts, 50 ms holds, yield_now, 3 step at once          p50   0.013 ms   p99   0.132 ms   max   1.195 ms
```

A second run on the same busy host gave p99s of 0.064, 18.706, 2.916, 1.046, 97.927 and 0.548 ms. These
two are models, made before the engine exists; targets 2 and 6 are measured on the real server in slice 6.

## Open questions

None. Decided by the owner on 2026-11-20 (q2649, and the two amendments in the issue, folded into this ADR and the README):

1. **Size.** The JavaScript subset, at its full size, not the JSON plan.
2. **`file.read` is guarded.** On a server with no guard a script reads terminals and nothing else.
3. **The caller is only what it says.** The guard gets it and may use it to refuse more (a reviewer's
   script may not act), never to allow more.
4. **The cap.** 20,300, in PR #205. This revision's 270 more lines leave 215 of its 485 to spare; the sum
   is in README.md, "Slices".

## `.drover/arch.toml`

This PR adds the layer, lets the server use it, and adds the bans that make "pure" a check; slice 1 adds
the fixture (`ci/fixtures/arch-script-pure`) that breaks them. The bans on processes, sleeping, the clock
and `env::var` already cover every crate. The layer uses `pattern`, the layer #193 adds for
`shed-pattern`, not `proto`: until #193 lands the name matches no crate, and slice 4, the first to use
it, waits for #193.

```toml
[[layer]]
name = "script"
paths = ["crates/shed-script/src/**"]
mods = ["shed_script"]
may_use = ["pattern"]

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

`scripts/dep-direction.sh` (`just deps`) gains one check: shed-script's dependencies are shed-pattern and
serde_json, nothing else. Two generated files join the list, each written by `just generate`: the gate's
counts in slice 3 (`just generate` gains `SHED_BLESS=1 cargo test -p shed-script --locked --test perf
counts`) and the skill's method page in slice 7:

```toml
generated = ["CHANGELOG.md", "docs/protocol.md", "docs/shed.openrpc.json", "crates/shed-script/tests/perf/counts.json", "skill/script/methods.md"]
```

No central list grows. The fixed scripts are a folder, one file each with its own ceiling, collected by
the test that reads the folder. New code is new files (`shed-proto/src/script.rs`,
`shed-server/src/script.rs` and `script/guard.rs`, `shed/src/script.rs`); the files many PRs edit get a
few lines each: a `Method` variant and its arms, one `dispatch` arm, one `Command` variant, one function
in `runner.rs`, two recipes' lines in the `justfile`.
