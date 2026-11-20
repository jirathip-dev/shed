# Script: shed's own code mode (#192)

## Summary

Decision: shed gets a `script` method. A caller sends one program in a JavaScript subset and the server runs it beside the terminals, in a new pure crate that shed builds itself (lexer, parser, compiler, a small virtual machine; no third-party engine, no new dependency). The program calls shed's methods through the same path a socket request takes. A call that acts is first shown to a guard: one process for the whole run, named in the server's config file, which answers each call in well under a millisecond; with no guard named the call is refused. Steps, memory, calls, logs and a deadline bound every run. Speed has six targets, each with its measurement, and a gate: CI compares fixed scripts' step counts and peak memory with a committed baseline, and the reviewer of slices 3 to 7 reports before and after times.
Slices: (1) parser, +1,700; (2) values and heap, +550; (3) compiler and virtual machine, +1,680; (4) built-ins, +950; (5) promises and the host interface, +400; (6) the server's `script` method and the guard, +620; (7) the command, `--on`, cancelling across machines, the skill doc and the numbers from a real remote host, +160. About +6,060 lines, 230 more than before this revision (the guard as one process, the collector taken in slices, the counters the gate reads). With the other queued work that is about 20,045 of the 20,300 in PR #205.
Open questions: none. The first three are decided as recommended and the cap by PR #205.

The question, the options with their line estimates, the decision and its consequences are in
[adr.md](adr.md), with the performance targets. This file has how it reads, the language, the shapes, the
boundaries, the statecharts, the errors, the guard, the limits, how the speed is measured and gated, the
tests and the slices. [check.mjs](check.mjs) runs the four examples below against a fixture host and fails
when a reply differs from the one shown (`node check.mjs`).

## How it reads

A lane, in one Bash call (the skill doc tells it how):

```console
$ shed script -f blocked.js                  # logs, then the result, then `[3 calls, 2 ms]`
$ shed script - < start.js                   # the program on stdin
$ shed --on box script -f errors.js --json   # runs on box's server; the whole reply as one line
$ shed script -f long.js --timeout 300       # the deadline, 60 s unless raised (at most 600)
```

drover, in place of N calls: one line after `hello`, one line back.

```json
{"jsonrpc":"2.0","id":2,"method":"script","params":{"code":"…","timeout":120,"caller":{"terminal":"orchestrator-7"}}}
```

A program is the body of an async function: `await` and `return` work at the top, and what it returns is
the reply's `result`. The three worked examples, with their replies for check.mjs's fixture:

```js
// blocked.js: every blocked lane of a fleet, with the last 15 lines of each
const lanes = await shed.ls({ labels: { fleet: "shed" } });
const blocked = lanes.filter((t) => t.reported?.state === "blocked");
return Promise.all(
  blocked.map(async (t) => {
    const { text } = await shed.read({ name: t.name, lines: 15 });
    return { name: t.name, kind: t.reported.kind ?? null, note: t.reported.note ?? null, tail: text.split("\n") };
  }),
);
```

```json
{"result":[
  {"name":"impl-190","kind":"permission","note":"Bash: cargo test",
   "tail":["Bash(cargo test --workspace --locked)","Do you want to proceed?","> 1. Yes   2. No"]},
  {"name":"impl-191","kind":"question","note":"Which cap raise?",
   "tail":["The slice needs 180 more lines than the cap has.","Raise the cap, or trim the CLI help?","> "]}],
 "logs":[],"calls":3,"ms":2}
```

Three calls, no guard (all reads); the two reads were in flight together. The fixture's screens are three
lines long, so `lines: 15` returns three.

```js
// start.js: run, send and wait for one lane in one trip
const lane = await shed.run({
  name: "impl-192",
  cwd: "/home/user/work/shed/issue-192",
  cmd: ["claude"],
  labels: { fleet: "shed", issue: "192", role: "impl" },
  idle_after: 5,
});
await shed.wait({ name: lane.name, until: ["idle"], timeout: 20 });
await shed.send({ name: lane.name, text: "Read .brief.md fully, then start." });
const { outcome, terminal } = await shed.wait({ name: lane.name, until: ["done", "blocked", "error"], timeout: 30 });
return { name: lane.name, pid: lane.pid, outcome, state: terminal.reported?.state ?? null, note: terminal.reported?.note ?? null };
```

```json
{"result":{"name":"impl-192","pid":4192,"outcome":"matched","state":"blocked","note":"Bash: git status"},
 "logs":[],"calls":4,"ms":13182}
```

The guard was started once, at `run` (20 ms in the fixture), and answered `send` on the same process in
under a millisecond; both waits count toward the 60 s deadline.

```js
// errors.js: a filter over read({history: true})
const { text } = await shed.read({ name: "impl-190", history: true });
const lines = text.split("\n");
const kept = lines
  .map((line, i) => ({ n: i + 1, line: line.trim() }))
  .filter(({ line }) => line.matches("^error\\[E\\d+\\]") || line.startsWith("test result: FAILED"));
console.log(`${lines.length} lines read, ${kept.length} kept`);
return kept.slice(-20);
```

```json
{"result":[
  {"n":412,"line":"error[E0308]: mismatched types"},
  {"n":1841,"line":"error[E0599]: no method named `settle` found for struct `Machine`"},
  {"n":2913,"line":"test result: FAILED. 212 passed; 1 failed; 0 ignored"}],
 "logs":["2913 lines read, 3 kept"],"calls":1,"ms":1}
```

2,913 lines stayed in the server; three came back. `matches` takes shed's own pattern (`wait --output`'s),
since the subset has no regex literals.

```js
// refused.js: the guard says no
await shed.kill({ name: "impl-190" });
return "killed";
```

```json
{"error":{"name":"GuardError","message":"kill refused by the guard: lanes are stopped with drover kill",
          "line":2,"column":12,"code":"GUARD_REFUSED","method":"kill"},
 "logs":[],"calls":1,"ms":21}
```

The server, `crates/shed-server/src/script.rs`: the method with its loop, then the host the engine calls
back. Handing the thread back is the server's, since the engine has no runtime:

```rust
pub(crate) async fn answer(registry: &Shared, outer: &Session, p: ScriptParams) -> Result<Value, RpcError> {
    let (started, deadline) = deadline(p.timeout)?;            // INVALID_PARAMS over 600 s
    let program = match Program::parse(&p.code) {
        Ok(program) => program,
        Err(error) => return reply(Err(error), None, started),  // SyntaxError, 0 calls
    };
    let guard = Guard::for_run(&lock(registry).config);        // `[script]`, read once; no process yet
    let host = ScriptHost { registry, outer, guard, caller: p.caller };
    let mut run = Run::new(&program, Limits::default(), &host);
    let outcome = tokio::time::timeout_at(deadline, async {
        let _slot = SLOTS.acquire().await;                      // 8 runs at once
        loop {
            let step = {
                let _stepping = STEPPING.acquire().await;       // the worker threads less one step at once
                run.step()                                      // at most 10,000 steps, or as much collecting
            };
            if let Step::Done(outcome) = step {
                break outcome;
            }
            run.settle().await;                                 // the calls in flight; waits when every task does
            tokio::task::yield_now().await;                     // the reactor and the other clients first
        }
    })
    .await
    .unwrap_or_else(|_| Err(run.stopped(Limit::Deadline)));     // the calls in flight are dropped
    reply(outcome, Some(&run), started)                         // logs and calls survive; the guard ends here
}

impl Host for ScriptHost<'_> {
    fn call(&self, method: &str, params: Value) -> Call<'_> {
        Box::pin(async move {
            if let Some(answer) = self.local(method, &params) { // `methods`, `describe`
                return answer;
            }
            let m = Method::parse(method).ok_or_else(|| no_such(method))?;
            match m.in_script() {                               // an exhaustive match in shed-proto
                Access::Refused => return Err(not_in_script(m)),
                Access::Guarded => self.guard.ask(m, &params, self.caller.as_ref()).await?,
                Access::Read => {}
            }
            let mut session = self.outer.inner();               // hello done, the outer read-only marks
            crate::dispatch(self.registry, &mut session, &Request::inner(m, params)).await
        })
    }
}
```

The guard, `crates/shed-server/src/script/guard.rs`: one process for the run, one line each way per call.

```rust
pub(crate) async fn ask(&self, m: Method, params: &Value, caller: Option<&Caller>) -> Result<(), RpcError> {
    let Some(config) = &self.config else { return Err(refused(m, NO_GUARD)) };
    let mut held = self.held.lock().await;                      // one call at a time, in call order
    let id = held.next_id();
    let line = serde_json::to_string(&Asked { id, method: m, params, machine: acts_on(m, params), caller })?;
    let answer = match config.per {
        Per::Run => held.exchange(config, &line).await,         // starts the process when none is alive
        Per::Call => once(config, &line).await,                 // the fallback: `runner::exec`, exit status
    };
    verdict(m, id, answer)                                      // anything but a plain allow is a GuardError
}
```

`Run` is plain data that is `Send`: `step` never blocks and never recurses natively, and `settle` is the only
future in the engine crate. An engine test drives the same loop with a fake `Host` and no runtime.

## The language

JavaScript, less what a 60-second program driving terminals does not need. Anything outside is a
`SyntaxError` at its line and column that names the construct and what to write instead.

| | in | out, with the error's hint |
|---|---|---|
| bindings | `const`, `let`; destructuring of objects and arrays with defaults and `...rest` | `var` (use `let`) |
| functions | `function`, arrow functions, `async`, default and rest parameters, closures | `function*`, `yield`, `this`, `class`, getters, setters, `new` (only `new Error(m)`) |
| statements | `if`/`else`, `for (const x of xs)`, `for (;;)`, `while`, `break`, `continue`, `return`, `throw`, `try`/`catch`; semicolons optional as in JavaScript | `switch` (use `if`), `do`, `for…in` (use `Object.keys`), `finally`, labels, `with`, `import`, `export` |
| literals | numbers, `'…'` `"…"`, template strings, `true` `false` `null` `undefined`, `[a, ...b]`, `{a, b: 1, [k]: v, ...o}` | regex literals (use `text.matches(pattern)`), `1n`, tagged templates |
| operators | `+ - * / % **`, `=== !== < <= > >=`, `== null` and `!= null`, `&& \|\| ?? !`, `?:`, `?.`, `typeof`, `await`, `= += -= *= /= ??= \|\|= &&=`, `++ --`, spread in calls | `==` and `!=` with anything but `null`, `in`, `instanceof`, `delete`, `void`, the comma and bitwise operators |
| nesting | 200 levels of brackets, blocks and functions | deeper is a `SyntaxError`, so no source can overflow the parser's stack |

The globals are exactly these; any other name is a `ReferenceError` (with a hint for the usual reaches:
`require`, `fetch`, `process`, `setTimeout`, `Date`, `Buffer`, `RegExp`, `globalThis`, `eval`).

| global | what it has |
|---|---|
| `shed` | `shed.<method>(params)` for every shed method by its wire name (`shed.read`, `shed.file.write`, `shed.machine.stats`), `shed.methods()`, `shed.describe(name)`; each returns a promise |
| `console` | `log`, `info`, `warn`, `error`, `debug`: one line of the reply's `logs` each; strings as they are, other values as JSON |
| `JSON` | `parse`, `stringify(value, null, indent)` |
| `Math` | `min max floor ceil round trunc abs sign sqrt pow`; no `random` (the engine has no entropy and no clock) |
| `Promise` | `all`, `allSettled`, `race`, `resolve`, `reject`; on a promise `then`, `catch` |
| `Object`, `Array`, `String`, `Number`, `Boolean`, `Error` | `Object.keys values entries fromEntries assign hasOwn`; `Array.isArray from`; `String(x)`, `String.fromBytes(bytes)`; `Number(x)`, `Number.isInteger isFinite`; `Boolean(x)`; `Error(message)` |
| `parseInt`, `parseFloat`, `isNaN`, `NaN`, `Infinity`, `undefined` | as in JavaScript |

| on | methods |
|---|---|
| a string | `length at charAt slice substring split trim trimStart trimEnd startsWith endsWith includes indexOf lastIndexOf toLowerCase toUpperCase replace replaceAll padStart padEnd repeat concat localeCompare`, and shed's two: `matches(pattern)` (the pattern of `wait --output`: `.`, `[…]`, `\d \w \s`, `* + ?`, `^ $`) and `toBytes()` |
| an array | `length at push pop shift unshift slice concat join includes indexOf lastIndexOf reverse flat entries`, and with a callback `map filter forEach find findIndex some every reduce flatMap sort` |

Where it differs from JavaScript it refuses rather than guesses: no implicit conversion (`"5" * 2`,
`1 < "2"` and `+` between a number and an object are `TypeError`s; `+` joins only when one side is a
string), and `replace` and `replaceAll` take a string, not a regex. Three things differ without an error,
and the skill doc says so: `sort` without a callback orders numbers as numbers; a string is its code
points, so `length` and indexes count those (the same as JavaScript for ASCII); an object or array in a
template string is its JSON.

`exec` and `file.read` return bytes as arrays of numbers, as on the wire: `String.fromBytes(r.stdout)` is
the text, and `text.toBytes()` the way back for `file.write` and `exec`'s `stdin`.

## Values and JSON

```rust
/// One value. `Copy`: everything that can grow lives in the heap, behind a handle.
pub enum Value { Undefined, Null, Bool(bool), Number(f64), Ref(Handle) }

enum Cell {
    Str(Box<str>),
    Array(Vec<Value>),
    Object(Fields),                      // string keys in insertion order; no prototype
    Closure { function: FnId, scope: Handle },
    Scope { parent: Option<Handle>, slots: Vec<Value> },
    Native(Builtin),                     // a built-in, or one bound to its receiver
    HostPath(Box<str>),                  // `shed.file` on its way to `shed.file.read(…)`
    Promise(Settled),                    // pending with its waiters, fulfilled, or rejected
    Task(Frames),                        // one async call: its frames and operand stack
}
```

The heap is an arena of cells with a mark and sweep from the tasks, the globals and the calls in flight.
Every cell is counted (32 bytes, plus its text, 16 per array slot, key bytes plus 16 per field), so "bytes
held by live values" is exact after a sweep and garbage costs nothing. A collection starts when the bytes
allocated since the last one pass what was live after it (at least 1 MiB), so its work is paid for by the
allocating that fuel already counts. It is a state of the machine, not a pause of the server: its mark
stack and sweep position are data, `step` takes its work in the same slices as steps, and the script's own
code waits while it runs, so it needs no write barrier. An error is an object: `name`, `message`, `line`,
`column`, and for a shed or guard error `code` and `method`.

| JSON | value | and back (call params, `result`, `JSON.stringify`) |
|---|---|---|
| `null`, `true`, `false`, a string | the same | the same |
| a number | a number (an f64: whole numbers are exact up to 2^53) | an integer when it is whole and within 2^53, so `lines: 15` reaches the server as `15`; else a float |
| an array | an array | the same; an `undefined` element is `null` |
| an object | an object, keys in the order received | the same; an `undefined` field is left out |
| | `NaN`, `Infinity`, a function, a promise, a `shed` path | a `TypeError` naming the path, `result[2].tail is a Promise: await it`. Only `JSON.stringify` follows JavaScript here (`null`, or left out) |

A call's result is counted into the heap as it is converted; a result that does not fit ends the run with
`MEMORY`.

## Data shapes

The wire, `crates/shed-proto/src/script.rs` (a new file: `lib.rs` is 60 lines under the size limit):

```rust
/// `script` params.
pub struct ScriptParams {
    /// The program: the body of an async function, in shed's JavaScript subset; at most 256 KiB.
    pub code: String,
    /// Stop it after this many seconds: 60 unless given, at most 600.
    pub timeout: Option<f64>,
    /// Run it on this machine's server, over its held link.
    pub machine: Option<String>,
    /// Who sends it, as the sender says; the guard gets it unchanged.
    pub caller: Option<Caller>,
}

/// What a script's sender says about itself. shed proves none of it.
pub struct Caller {
    /// The terminal it runs in (`shed script` sends `$SHED`).
    pub terminal: Option<String>,
    /// Its working directory.
    pub cwd: Option<PathBuf>,
}

/// `script` result: `result` when the program returned, `error` when it did not; never both.
pub struct ScriptResult {
    /// What the program returned, as JSON; `null` when it returned nothing.
    pub result: Option<Value>,
    /// Why it stopped.
    pub error: Option<ScriptError>,
    /// One entry per `console` call, in order.
    pub logs: Vec<String>,
    /// How many `shed` calls it made, refused ones included.
    pub calls: u64,
    /// How long it took, queueing and guards included.
    pub ms: u64,
}

/// A script's error: where, and what.
pub struct ScriptError {
    /// `SyntaxError`, `ReferenceError`, `TypeError`, `RangeError`, `Error`, `ShedError`, `GuardError` or `LimitError`.
    pub name: String,
    /// One line.
    pub message: String,
    /// The line, from 1.
    pub line: u32,
    /// The column, from 1, in code points.
    pub column: u32,
    /// A shed error's name (`NO_SUCH_TERMINAL`), a guard's verdict (`GUARD_REFUSED`) or the limit (`FUEL`).
    pub code: Option<String>,
    /// The shed method that failed or was refused.
    pub method: Option<String>,
}

/// How a method may be called inside a script.
pub enum Access { Read, Guarded, Refused }

impl Method {
    /// A `match` with no `_` arm: a new method does not compile until it is classified.
    pub const fn in_script(self) -> Access { … }
}
```

`Method::Script` is the 33rd method. `Pattern` moves from shed-core to `crates/shed-proto/src/pattern.rs`:
its only user is the server's `wait`, its syntax is part of the wire contract, and the engine may use
proto only.

The engine, `shed-script`:

```rust
/// What a script can reach: shed's methods, by wire name. The engine's only way out.
pub trait Host: Sync {
    /// Starts one call. The engine polls it with the others in flight and drops it to cancel.
    fn call(&self, method: &str, params: serde_json::Value) -> Call<'_>;
}
pub type Call<'a> = Pin<Box<dyn Future<Output = Result<serde_json::Value, RpcError>> + Send + 'a>>;

pub struct Program { /* compiled code and constants; no state */ }
impl Program { pub fn parse(code: &str) -> Result<Self, ScriptError>; }

pub struct Run<'h, H: Host> { /* the machine (heap, tasks, logs, counters) and its calls in flight */ }
impl<'h, H: Host> Run<'h, H> {
    pub fn new(program: &Program, limits: Limits, host: &'h H) -> Self;
    pub fn step(&mut self) -> Step;               // one slice: 10,000 steps or units of collecting
    pub async fn settle(&mut self);               // starts the slice's calls (32 at once), takes the answers
                                                  // that are ready; after `Waiting`, waits for one
    pub fn stopped(&mut self, limit: Limit) -> ScriptError;    // at the main task's position, naming calls in flight
    pub fn logs(&self) -> &[String];
    pub fn counts(&self) -> Counts;
}
pub enum Step { More, Waiting, Done(Result<serde_json::Value, ScriptError>) }

pub struct Limits { pub fuel: u64, pub memory: usize, pub depth: usize, pub calls: u64, pub logs: usize }
pub enum Limit { Fuel, Memory, Depth, Calls, Logs, Deadline, Reply }

/// What a run cost, exactly: the same program and answers give the same counts on every machine.
pub struct Counts {
    pub steps: u64,                // instructions, and what built-ins add
    pub peak_bytes: usize,         // the most that was live after a collection, or at the end
    pub collections: u64,
    pub largest_collection: u64,   // the cells and slots the largest one visited
    pub calls: u64,
}
```

The guard's table in the server's config file (`config.toml`, beside `[machines]`):

```toml
[script]
guard = ["drover", "guard"]     # started at a run's first guarded call: a line in and a line out per call
guard_timeout = 10              # seconds for one answer, the first with the start; 10 unless given
# guard_per = "call"            # the fallback, for a plain hook: a process per call, its exit status the answer
```

## Module boundaries

| module | lines | owns | may use |
|---|---|---|---|
| `shed-script`: `lex.rs` | 430 | tokens with line and column, template strings, the newline rule for semicolons | nothing |
| `ast.rs`, `parse.rs`, `error.rs` | 1,270 | the tree, the subset's edge (every "out" row above is one message here), the nesting limit | `lex` |
| `value.rs`, `heap.rs` | 550 | `Value`, `Cell`, the arena, the mark and sweep taken in slices, the byte count | nothing |
| `compile.rs` | 860 | tree to instructions: scopes, jumps, handlers, destructuring | `ast`, `value` |
| `vm.rs` | 930 | frames, tasks, promises, the step loop, fuel and depth, `Counts` | `heap`, `compile`, `builtins` |
| `builtins/`, `prelude.js`, `json.rs` | 950 | the tables above; JSON in and out. Methods that take a callback (`map`, `sort`, `then`) are 120 lines of the subset itself, compiled once per process, so no built-in ever calls back into the machine | `heap`, `value`, `shed_proto::Pattern` |
| `lib.rs` | 290 | `Program`, `Run` (the machine and its calls in flight), `Limits`, `Host` | all of the above, `shed_proto::{RpcError, ScriptError}`, `serde_json`, `std::future` |
| `shed-proto`: `script.rs`, `pattern.rs` | 100 | the wire shapes, `Access`, `Method::in_script`; `Pattern`, its 105 lines moved from shed-core | nothing |
| `shed-server`: `script.rs` | 255 | `answer` and its loop, `ScriptHost`, the run slots and the stepping permits, the deadline, the reply and its size check, `methods` and `describe` from `proto::openrpc()` | `shed_script`, `crate::dispatch`, `script::guard`, `clock` |
| `script/guard.rs` | 190 | `[script]` of the config file; the run's guard process, the line each way and its id; the per-call fallback; the verdict | `runner::held`, `runner::exec`, `toml` |
| `lib.rs`, `links.rs`, `machines.rs`, `runner.rs` | 155 | one `dispatch` arm; the peer-closed watch in `connection`; `script` forwarded on a connection of its own; `[script]` skipped by the machines reader; `exec` ends its group when dropped; `runner::held`, a piped process in a group of its own that ends when dropped | as today |
| `shed` (CLI): `script.rs` | 80 | `shed script`, its output and exit code | `shed_server::{Client, proto}` |

- `shed-script` is pure: no socket, file, process, thread, clock, environment or `unsafe`, and no async
  runtime. Its dependencies are `shed-proto` and `serde_json`. The layer rule, eight bans and the
  dependency check that hold this are at the end of [adr.md](adr.md).
- The engine knows no method name. `shed.a.b(p)` is `host.call("a.b", p)`; what exists, what is refused
  and what is guarded is the server's `ScriptHost` and proto's `Access`.
- An inner call is `dispatch`, the function a socket request reaches after its line is parsed, with a
  session of its own. It is polled inside the script's request, never spawned, so the request's save
  generation (`registry::answering`) and its handover count cover it: the reply is written after the state
  file has what the script changed, as for one call.
- Only `script/guard.rs` starts the guard, through `runner::held` (or `runner::exec` for a per-call guard).
  Only `script.rs` reads time, through `clock::tokio_now` and tokio's timeout, and only it hands the thread
  back: the engine never wakes itself.

## Statecharts

One run:

```mermaid
stateDiagram-v2
    [*] --> Parsing: script
    Parsing --> Replied: outside the subset / SyntaxError, 0 calls
    Parsing --> Queued: compiled
    Queued --> Stepping: one of 8 run slots is free
    Stepping --> Stepping: 10,000 steps, or as much collecting, used / hand the thread back, take the answers that are ready
    Stepping --> Waiting: every task waits on a call
    Waiting --> Stepping: a call settles / the tasks awaiting it are ready
    Stepping --> Draining: the body returned, calls it did not await are in flight
    Draining --> Replied: the last one settles / result, or the first one's error
    Stepping --> Replied: the body returned, nothing in flight / result
    Stepping --> Replied: uncaught throw / its error, calls in flight dropped
    Stepping --> Replied: fuel, memory, depth, calls or logs used up / LimitError, calls in flight dropped
    Queued --> Replied: deadline
    Stepping --> Replied: deadline
    Waiting --> Replied: deadline / LimitError DEADLINE naming the calls in flight, which are dropped
    Draining --> Replied: deadline
    Waiting --> Cancelled: the peer closed its socket / calls in flight dropped, no reply
    Stepping --> Cancelled: the peer closed its socket
    Draining --> Cancelled: the peer closed its socket
    Replied --> [*]: result or error with logs, calls and ms / over REPLY_LIMIT a LimitError instead
    Cancelled --> [*]
```

One call, from `shed.m(params)` to its promise settling. `Promise.all` over N calls is N of these at once
(32 in flight, the rest queued in order) and one task waiting for all of them, or for the first rejection:

```mermaid
stateDiagram-v2
    [*] --> Started: shed.m(params) / params to JSON, calls + 1, a pending promise
    Started --> Rejected: no such method, or refused in a script / ShedError
    Started --> Guarding: a guarded method
    Started --> Dispatching: a read
    Guarding --> Dispatching: the guard allows it
    Guarding --> Rejected: it says no, ends, answers out of turn, cannot start, takes too long, or none is configured / GuardError
    Dispatching --> Fulfilled: result / JSON to a value, counted
    Dispatching --> Rejected: shed's error / ShedError with its code
    Guarding --> Dropped: deadline, failure, limit, cancel / the run's guard is ended with the run
    Dispatching --> Dropped: deadline, failure, limit, cancel / the future is dropped
    Fulfilled --> [*]
    Rejected --> [*]
    Dropped --> [*]
```

The run's guard, one process (with `guard_per = "call"` there is no such state: each call starts its own):

```mermaid
stateDiagram-v2
    [*] --> NotStarted: a run starts / no process yet, and a run of reads never starts one
    NotStarted --> Answering: a guarded call / start the command, write the call's line
    NotStarted --> NotStarted: it cannot start / GUARD_FAILED
    Answering --> Idle: a line with the call's id / allow, or GUARD_REFUSED with its reason
    Answering --> NotStarted: it ends, or its line is not this call's answer / GUARD_FAILED, its group ended
    Answering --> NotStarted: no line within guard_timeout / GUARD_TIMEOUT, its group ended
    Idle --> Answering: the next guarded call, in call order / write its line
    Idle --> [*]: the run ends / its stdin closed, its group ended
    Answering --> [*]: the run ends
```

- A run steps only while it holds one of the stepping permits (the server's worker threads less one, so 3
  of 4), taken for each slice and given back before it waits or yields: scripts never hold every worker,
  whatever a slice costs.
- A call starts when it is called, not when it is awaited, as in JavaScript. A run that returns waits for
  the calls it started and did not await (`Draining`), so a forgotten `await` before `shed.send` still
  sends; if such a call fails, its error is the reply's.
- A dropped call may or may not have taken effect: a `send` half written, an `exec` whose group was just
  ended. The error names the methods that were in flight. What ran before stays done; nothing is rolled
  back.
- The peer closing its whole socket cancels, within a second; a peer that only finished writing does not (drover's remote
  call is `shed connect` with the request on stdin, which closes that side at once). The two are told apart
  in adr.md, "Checked by running".

## Errors

Everything about the program is in the result, as `exec` answers with an exit code: a failed script is
`{error, logs, calls, ms}`, so its logs survive. An RPC error is left for what happens before a run
(`INVALID_PARAMS` for a timeout over 600 or code over 256 KiB, `NO_LINK` for the machine).

| `name` | when | `code` | caught by `catch` |
|---|---|---|---|
| `SyntaxError` | the code is outside the subset; nothing ran | | no |
| `ReferenceError`, `TypeError`, `RangeError` | an unknown name; a wrong type, a property of `null`, a value JSON cannot carry; a bad pattern or count | | yes |
| `Error` | `throw new Error(m)`; any other thrown value, its text as the message | | yes |
| `ShedError` | a shed method failed, or may not be called in a script | shed's own name: `NO_SUCH_TERMINAL`, `INVALID_PARAMS`, `DRAFT_HELD`, …; `NOT_IN_SCRIPT`; `NO_SUCH_METHOD` | yes |
| `GuardError` | the guard did not allow a call | `GUARD_REFUSED` (it said no), `GUARD_FAILED` (it could not start, ended, or did not answer this call), `GUARD_TIMEOUT`, `NO_GUARD` | yes |
| `LimitError` | a limit ended the run | `FUEL`, `MEMORY`, `DEPTH`, `CALLS`, `LOGS`, `DEADLINE`, `REPLY_LIMIT` | no |

`line` and `column` are where it was thrown: for a failed call, the method's name in `shed.kill(`; for
`DEADLINE`, where the main body was waiting; for `REPLY_LIMIT`, its `return`. Messages are written to be
acted on in one more trip:

```text
SyntaxError 3:14: a regex literal is not in the subset: use text.includes(…) or text.matches(pattern)
ReferenceError 1:1: setTimeout is not defined: a script has no timers; shed.wait({…, timeout}) waits
TypeError 7:10: result[1].tail is a Promise: await it (Promise.all for a list)
ShedError 4:22: read: NO_SUCH_TERMINAL: impl-19
GuardError 2:12: kill refused by the guard: lanes are stopped with drover kill
GuardError 2:12: kill refused: this server has no script guard ([script] guard in config.toml)
LimitError 9:3: stopped at the 60 s deadline; in flight: wait, wait
LimitError 12:1: the reply is 9,437,201 bytes, over the limit of 8,388,608: return less
```

## The guard

| | |
|---|---|
| Where | `[script]` in the server's `config.toml`: `guard`, a command line; `guard_timeout` (10 s); `guard_per`, `"run"` unless given. Read once when a run starts, never from the request: a caller cannot name its own guard |
| The process | One for the run, started at its first guarded call (a run of reads starts none) in a process group of its own, its stderr the server's. It ends with the run: its stdin is closed and its group ended. A guard that has ended is started again at the next guarded call |
| Asked | Before each call of a guarded method, every time; nothing is remembered between calls. One line on its stdin, `id` first: `{"id":7,"method":"exec","params":{"cmd":["git","push"],"cwd":"/home/user/project"},"machine":null,"caller":{"terminal":"impl-190","cwd":"/home/user/work/shed/issue-190"}}`. `params` are the call's, untouched; `machine` is where the call will act when it names one (`machine`, or `box/t1`), else `null`; `caller` is the request's |
| Answers | One line on its stdout: `{"id":7,"allow":true}`, or `{"id":7,"allow":false,"reason":"lanes are stopped with drover kill"}`. One call is asked at a time, in call order, so a guard is a loop: read a line, write a line. The calls of one `Promise.all` are asked one after another and then run together |
| Allowed | Only by a line that parses, carries this call's `id` and says `"allow":true`, within the timeout |
| Refused | `"allow":false`: `GUARD_REFUSED` with its `reason` (at most 400 characters). It cannot start, ends before it answers, or its line is not JSON or carries another `id`: `GUARD_FAILED`. No line within `guard_timeout`: `GUARD_TIMEOUT`. After either of the last two its group is ended, so a guard that lost its place is never asked again. No `guard` key: `NO_GUARD` |
| In the script | A `GuardError` thrown at the call, naming the method and the reason; the method was never dispatched |
| The fallback, `guard_per = "call"` | For a guard that is a plain hook and cannot stay up: a process for each call, the same line on its stdin and then the end of input. Allowed when it exits 0 and wrote nothing to stdout; any other exit is `GUARD_REFUSED`, the reason its stderr's last line, else `exit 3`; anything on stdout is `GUARD_FAILED`. It costs a process a call (46 ms here for a Python hook, [guard-trip.py](guard-trip.py)) |
| A wrong `guard_per` | Refuses, both ways. A per-run guard asked per call writes its answer to stdout: `GUARD_FAILED`, the message naming the setting. A hook asked per run ends without a line, or waits for the end of input until the timeout |

| class | methods | in a script |
|---|---|---|
| read | `ls`, `get`, `read`, `wait`, `machine.stats`, `machine.links`, `agents`, `agents.match`, `rpc.discover` | called as they are; work with no guard configured |
| guarded | `run`, `send`, `keys`, `kill`, `rm`, `resize`, `state`, `acp.model`, `exec`, `file.write`, `file.read` (it changes nothing but reads any file the server's user can: the owner's decision) | the guard first |
| refused | `events`, `attach`, `screen.watch`, `terminal.input`, `terminal.keys`, `terminal.resize`, `terminal.readonly`, `handover`, `handover.check`, `hello`, `heartbeat`, `emit`, `script` | `ShedError` `NOT_IN_SCRIPT`: they stream, belong to a person's keyboard or to the connection, or replace the server |

A script runs under the guard of the server it runs on: with `--on box`, box's `[script]`. A call it
forwards to another machine (`machine`, `box/t1`) is guarded here, with that machine named, and arrives
there as a plain call. `wait` is a read and counts toward the deadline. `shed.methods()` returns `[{name, summary, script}]` with
`script` one of the three classes; `shed.describe(name)` returns that method's entry of the OpenRPC document
with the schemas it refers to, so one call is enough to write a correct one.

## Limits and the reply

| limit | value | counted by | over it |
|---|---|---|---|
| deadline | 60 s; `timeout` raises it to at most 600 | the server, with tokio; it starts when the request is read | `DEADLINE` |
| steps (fuel) | 50,000,000 | the engine: one per instruction; a built-in adds one per element or 64 bytes it touches. Collecting is not fuel: allocation, which is, pays for it | `FUEL` |
| live values | 64 MiB | the engine's heap, exact after a sweep; a built-in reserves before it allocates | `MEMORY` |
| call depth | 200 frames in a task | the engine | `DEPTH` |
| calls | 2,000, refused ones included | the engine | `CALLS` |
| calls in flight | 32; more wait their turn | the engine's driver | none |
| logs | 64 KiB | the engine | `LOGS`; the reply has the logs up to it |
| code | 256 KiB | the server | `INVALID_PARAMS` |
| runs at once | 8 per server; more wait inside their deadline | the server | `DEADLINE` |
| runs stepping at once | the server's worker threads less one (3 of 4; 1 on a single core) | the server, a permit per slice | none: a run waits its turn, a slice at a time |
| reply | `REPLY_LIMIT`, 8 MiB as one line | the server | `REPLY_LIMIT`: an error that says to return less, with the logs; never a cut reply |

The reply is `{result, logs, calls, ms}`, or `{error, logs, calls, ms}`. Only the deadline is the
caller's to set. The engine reads no clock, so the same program with the same answers takes the same
steps on every machine, and the fuel and memory tests are exact.

## Measuring and the gate

The six targets are in [adr.md](adr.md), "Performance". This is what measures them and what stops a
regression. Nothing in CI reads a clock: the engine is deterministic, so CI compares counts, and times are
a reviewer's, taken on a fleet host.

The fixed scripts, `crates/shed-script/tests/perf/`: one file each, collected by the folder, no list. A
script's first line is its ceiling (`// ceiling: steps 5500000, peak_bytes 65536`); its calls are answered
from `<name>.host.json`.

| script | what it does | what it measures | from slice |
|---|---|---|---|
| `spin.js` | a counted loop of arithmetic and a function call: 5,000,000 steps | steps a second | 3 |
| `ten-kb.js` | the three worked examples repeated to 10,240 bytes; parsed and compiled, not run | parse and compile time | 3 |
| `hold.js` | keeps 60 MiB of small objects and arrays, then makes 200 MiB of garbage | the collector: the longest slice, the longest collection | 3 |
| `shapes.js` | 20,000 objects through `map`, `filter`, `sort`, template strings, destructuring, closures | steps a second where values are made | 4 |
| `big.js` | `split`, `map` and `join` over an 8 MiB answer, the largest a call can give | the longest hold by one built-in, which is not sliced | 4 |
| `fan.js` | 2,000 calls, 32 in flight, answered at once | the engine's own cost of a call | 5 |
| `blocked.js`, `start.js`, `errors.js` | the worked examples | the counts of what lanes send | 5 |

| level | what | where it runs | it fails when |
|---|---|---|---|
| exact | `counts.json` beside the scripts holds each one's `Counts` (steps, peak bytes, collections, the largest collection, calls). It is generated: `just generate` gains `SHED_BLESS=1 cargo test -p shed-script --locked --test perf counts`, as `protocol_doc` writes the protocol docs, and `.drover/arch.toml` lists the file under `generated` | CI: `just arch`, and `counts_match_the_baseline` in `just test` | a change moved a count and the file was not regenerated. Regenerated, every count that moved is a line of the PR's diff |
| ceiling | the first line of each script, written by hand at its first counts plus 10%; `just generate` never writes it | CI: `counts_stay_under_their_ceilings` in `just test` | steps or peak bytes pass the ceiling. Raising one is a hand edit a reviewer sees, with its reason in the PR body |
| times | `just script-bench`: `speed_report` of `crates/shed-script/tests/perf.rs` and, from slice 6, of `crates/shed/tests/script.rs`, both `--release -- --nocapture`. One line per target: the value, the target, the range of 5 runs. `scripts/script-remote.sh <machine>` for target 1 | a fleet host, by impl and by rev: slices 3 to 7 are queued `--perf` | rev's `Perf:` block shows a target missed by more than the runs' range. A target changes only by the owner's answer |

What each `--perf` slice shows. Its first commit is `baseline:`, the command and its numbers on the base;
a row the base cannot run yet is new, and is held to its target instead.

| slice | before, in its `baseline:` commit | after, in rev's `Perf:` block |
|---|---|---|
| 3 | adds the report, `spin.js`, `ten-kb.js`, `hold.js`; times what the base has: slice 1 parsing `ten-kb.js`, slice 2's heap filled and collected as `hold.js` does it, from Rust | targets 3, 4 and 5: `spin.js`, `ten-kb.js` parsed and compiled, `hold.js` run as a script. `counts.json` is first written |
| 4 | the report on the base | the same rows within their range (the prelude may not slow `spin.js` or `ten-kb.js`); new: `shapes.js`, and `big.js` at most 30 ms in one hold |
| 5 | the report on the base | the same rows; new: `fan.js` at most 20 µs a call, and the examples |
| 6 | adds the server's report; before is today's way on one host: `ls` and 8 `read`s as 9 `shed` commands (`shed ls` is 7 ms here, and returns 812 KB the caller reads whole) | one `script` request for the same, at most 1.5 times one `ls`; target 2, the guard per run against per call; target 6, another client's read |
| 7 | the per-call half of `scripts/script-remote.sh`, on a real remote host | target 1: sessions and wall time both ways. The PR body restates all six targets with their numbers, the end-to-end record |

`scripts/script-remote.sh <machine>` makes 8 terminals there labelled `fleet=perf`, each reported blocked,
then runs both ways 10 times with no link held, so each `shed --on <machine>` call is its own `shed connect`
over SSH, as drover's is today: `ls` then a `read` for each lane, and `script -f blocked.js`. An `ssh` shim
first on `PATH` counts the sessions. It prints the median and the range of each, and removes the terminals.

## The CLI and the skill doc

```console
shed script -f <file> [--timeout <S>]     # the file is the program
shed script - [--timeout <S>]             # stdin is the program
shed --on <machine> script …              # on that machine's server, one round trip
```

Without `--json`: each log line, then the result (a string as it is, anything else as indented JSON) on
stdout, then `[N calls, X ms]` on stderr; exit 0. On an error: the logs, then `error: <name> <line>:<column>:
<message>` on stderr; exit 1. With `--json`: the reply as one line, and the same exit code. It sends
`caller` as `$SHED` and its working directory. With `--on`, the command goes to the local server with
`machine` when that holds the machine's link, which forwards it on a connection of its own (the shared one
would hold every other call to that machine for the script's whole run); else it goes over `ssh … shed
connect`, as `exec` does.

`skill/script/`, laid out as blocks' skill and read by a lane in its brief:

| file | what |
|---|---|
| `SKILL.md` | 60 lines: when a script pays (three or more calls, or a read you need a part of); the form (an async function's body, `return` the answer, `console.log` to trace); `shed.<method>(params)`, `methods`, `describe`; the limits in one table; what is not JavaScript here, as the "out" column; how a refusal and a limit read |
| `methods.md` | generated by `just generate` from the OpenRPC document: each method, its one line, its class. Listed under `generated` in `.drover/arch.toml` |
| `examples/blocked.js`, `start.js`, `errors.js` | the three above. A test in `crates/shed/tests/script.rs` runs each against a real server, so they cannot rot |

## Tests that prove the limits

Engine tests need no runtime and no clock (`crates/shed-script/tests/`, a fake `Host`); server tests drive
the real binary over its socket (`crates/shed/tests/script.rs`).

| test | proves |
|---|---|
| `a_busy_loop_stops_on_fuel` | `while (true) {}` is `FUEL` after exactly 50,000,000 steps, taken in slices, in a test with no runtime and no clock |
| `growing_values_stop_on_memory`, `garbage_is_not_counted` | an array grown without end is `MEMORY` before `FUEL`; a loop that builds and drops 1 GiB of strings finishes |
| `deep_recursion_stops_on_depth`, `deep_nesting_is_a_syntax_error` | `DEPTH` at frame 201; 100,000 nested brackets are a `SyntaxError`, not a stack overflow |
| `a_hung_wait_stops_at_the_deadline` | a `wait` on a terminal that never reports is `DEADLINE` at 60 s of a paused clock, names `wait`, and the wait is gone |
| `closing_the_connection_cancels_the_calls_in_flight`, `a_peer_that_only_stops_writing_gets_its_reply` | after the socket closes, the `exec` the script was waiting on has no process left within 2 s and its marker file never appears; `shed connect` with the request on stdin still prints the reply. Run on Linux and macOS |
| `only_the_listed_globals_exist` | the global table equals the list above; `require`, `process`, `fetch`, `globalThis`, `Date`, `setTimeout`, `eval`, `Function`, `Symbol`, `Reflect`, `Proxy` are each a `ReferenceError` |
| `a_guard_that_says_no_refuses`, `a_guard_that_exits_non_zero_refuses`, `…_cannot_start_refuses`, `…_times_out_refuses`, `…_answers_out_of_turn_refuses`, `no_guard_refuses_what_acts`, `a_read_needs_no_guard` | each `GuardError` code, with a per-run guard and with `guard_per = "call"` (a per-run guard that exits non-zero has ended without its answer); the refused `exec` left no process and no event; `ls` and `read` answer on a server with no `[script]` |
| `one_guard_process_serves_a_run`, `a_run_of_reads_starts_no_guard`, `the_guard_ends_with_its_run`, `a_wrong_guard_per_refuses` | 50 guarded calls reach one process, which is gone when the reply is; a per-run guard set per call and a hook set per run both refuse |
| `every_method_has_a_class` | the table in this file equals `Method::in_script` (which also cannot compile without a new method's arm) |
| `the_2001st_call_stops_the_run`, `logs_stop_at_64k`, `a_reply_over_the_limit_is_an_error_not_a_cut` | `CALLS`, `LOGS` with the logs kept, `REPLY_LIMIT` with a reply that parses |
| `promise_all_runs_its_calls_at_the_same_time` | both calls started before either settled; with 40, at most 32 were in flight |
| `an_inner_call_is_a_socket_call` | a `run` from a script emits `started`, is in the state file before the reply is written, and fails with the same `INVALID_PARAMS` text as over the socket |
| `an_unawaited_send_is_sent` | the run drains; its failure is the reply's error |
| `counts_match_the_baseline`, `counts_stay_under_their_ceilings` | the gate of "Measuring and the gate": each fixed script's steps, peak bytes and collections equal `counts.json` and stay under its ceiling, twice in a row and on Linux and macOS |
| `no_slice_is_over_10000_units` | with `hold.js` and `spin.js`, no `Run::step` does more than 10,000 steps or units of collecting: a collection of 60 MiB is hundreds of slices, never one |
| `scripts_never_hold_every_worker` | while 8 scripts spin, at most the worker threads less one are inside `step` at any moment |

## Slices

Each is its own issue and PR, in this order; lines are non-test lines under `crates/*/src`, and "total" is
where the slice leaves the count with nothing else merged. Staging (`48d5312`) is at 13,248. Slices 3 to 7
are queued `--perf`.

| | slice | what | lines | total |
|---|---|---|---|---|
| 1 | **Parser** | the `shed-script` crate; `lex`, `ast`, `parse`, `error`; every "out" construct's message; the layer, the bans, their fixture and the dependency check | +1,700 | 14,948 |
| 2 | **Values and heap** | `value`, `heap`: cells, the arena, the byte count, the mark and sweep as a state taken in slices | +550 | 15,498 |
| 3 | **Compiler and machine** | `compile`, `vm` without promises, `Program`, `Run::step`, `Counts`; fuel and depth; closures, destructuring, `try`/`catch`; the fixed scripts, `counts.json`, `just script-bench` | +1,680 | 17,178 |
| 4 | **Built-ins** | `builtins/`, `prelude.js` compiled once, `json`; `Pattern` moves to shed-proto (no net lines); memory reserved before allocating; the globals test | +950 | 18,128 |
| 5 | **Promises and the host** | tasks, `await`, `Promise.*`, `Host`, `Run::settle`, the call and in-flight limits, draining | +400 | 18,528 |
| 6 | **The server** | `ScriptParams` and friends, `Access`, `Method::Script`; `script.rs` with its loop and the stepping permits; `script/guard.rs`, the guard per run and per call, `[script]`; `runner::held`; the peer-closed watch; `exec` ends its group on drop; the two protocol docs regenerate | +620 | 19,148 |
| 7 | **The command and the skill** | `shed script`; `script` over a link on its own connection, ended with the run; `shed connect` ends when its reader is gone (rustix's `event` feature, on in shed-core already); `skill/script/` with `methods.md` generated; the examples' test; `scripts/script-remote.sh` and its numbers | +160 | 19,308 |

Slices 1 to 5 touch no existing file but `Cargo.toml`, `.drover/arch.toml`, the `justfile` (in 3) and, in
4, the `Pattern` move, so they conflict with nothing in flight. The estimate is 6,060: 230 over the 5,830
of this ADR's first version, for the guard as one process (+105), `runner::held` (+30), the server's loop
and permits (+25), the collector in slices (+40) and the counters (+30). PR #205 raises the cap to 20,300
for staging's 13,248, #194's 47, the client slices' 690 and 5,830 of this, with 485 to spare; with 6,060
that is 20,045 and 255 to spare. The last design's largest slice landed 43% over its own estimate (#178:
472 for 330), so a slice here may well cross it; the one that would trims first, then asks with
`drover cap`'s view, as the cap rule says. drover's side (its guard command, its generated scripts) is
drover's issue, after slice 7.
