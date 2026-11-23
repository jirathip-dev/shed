---
name: shed-script
description: Drive many shed terminals in one round trip with `shed script`, a program in a JavaScript subset that the shed server runs beside the terminals. Use when a task needs three or more shed calls, calls that depend on each other, or only part of a long read.
---

# shed script

One Bash call instead of N: the server runs the program, makes the calls, and sends back only what it returns.
It pays off from three calls, or when you need a few lines of a long screen or history.

```console
$ shed script -f blocked.js               # logs, then the result; `[3 calls, 2 ms]` on stderr
$ shed script - < start.js                # the program on stdin
$ shed --on box script -f errors.js --json   # on box's server; the whole reply as one line
$ shed script --check -f blocked.js       # parse and compile only: nothing runs, no call
$ shed script -f long.js --timeout 300    # the deadline: 60 s unless raised, at most 600
```

## The form

- A program is the body of an async function: `await` and `return` work at the top, and what it returns
  is the result. `console.log` lines come back in `logs`, failed runs included; use them to trace.
- `shed.<method>(params)` calls a method by its wire name (`shed.read`, `shed.file.write`) and returns a
  promise. `await shed.methods()` lists every method with its class; `await shed.describe("read")` gives
  one method's params and result. The list is [methods.md](methods.md).
- `Promise.all(list.map(async (t) => …))` runs calls together (32 in flight at most).
- A call that acts (`run`, `send`, `kill`, `exec`, `file.read`, …) is first asked of the guard named in
  the server's `[script]` config; with none it is refused. Reads (`ls`, `get`, `read`, `wait`) never are.
  `--role R` tells the guard your role; it can only make it refuse more.
- The worked examples are [examples/](examples/): `blocked.js`, `start.js`, `errors.js`.

## The limits

| limit | value | over it |
|---|---|---|
| deadline | 60 s, `--timeout` up to 600 | `DEADLINE` |
| steps | 50,000,000 | `FUEL` |
| live values | 64 MiB | `MEMORY` |
| call depth | 200 frames | `DEPTH` |
| calls | 2,000, refused ones included | `CALLS` |
| logs | 64 KiB | `LOGS` |
| code | 256 KiB | refused before it runs |
| reply | 8 MiB | `REPLY_LIMIT`: return less; never cut |

## Not JavaScript here

Each of these is a `SyntaxError` or `ReferenceError` whose message says what to write instead: `var`,
`class`, `this`, `new` (but `new Error(m)`), generators, `switch`, `do`, `for…in`, `finally`, labels,
`import`, regex literals (use `text.includes(…)` or `text.matches(pattern)`, the pattern of
`wait --output`), `==` with anything but `null`, `in`, `instanceof`, `delete`, bitwise operators,
`require`, `fetch`, `process`, `setTimeout`, `Date`, `Math.random`. There is no implicit conversion:
`"5" * 2` is a `TypeError`, and `+` joins only when one side is a string.
Three things differ without an error: `sort()` orders numbers as numbers; a string counts code points;
an object in a template string is its JSON. `exec` and `file.read` give bytes as arrays of numbers:
`String.fromBytes(r.stdout)` is the text, `text.toBytes()` the way back.

## How a failure reads

The command exits 1 and prints `error: <name> <line>:<column>: <message>` after the logs:

```text
error: SyntaxError 3:14: a regex literal is not in the subset: use text.includes(…) or text.matches(pattern)
error: ShedError 4:22: read: NO_SUCH_TERMINAL: impl-19
error: GuardError 2:12: kill refused by the guard: lanes are stopped with drover kill
error: LimitError 9:3: stopped at the 60 s deadline; in flight: wait, wait
```

`ShedError` and `GuardError` can be caught with `try`/`catch`; a limit cannot. Nothing is rolled back:
`--json`'s `acting` lists every call that acted, with its result or refusal, so you know what was done.
