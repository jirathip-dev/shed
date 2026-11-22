wire: script
params: ScriptParams
result: ScriptResult
on: machine

Run a program in shed's JavaScript subset (the body of an async function, at most 256 KiB) beside the terminals: `shed.<method>(params)` calls a method by its wire name through the same path a request takes, and what the program returns is `result`.
A read (`ls`, `get`, `read`, `wait`, `machine.stats`, `machine.links`, `agents`, `agents.match`, `rpc.discover`, `terminal.procs`) is called as it is; any other method that acts is first shown to the guard of `[script]` in the server's config file (one process for the run, one JSON line each way per call, or with `guard_per = "call"` a process per call), and with none configured it is refused; streams, a person's input and the connection's own methods are refused (`NOT_IN_SCRIPT`).
`caller` goes to the guard unchanged.
Steps, live memory, calls, logs and the deadline (`timeout`, 60 s, at most 600) bound a run.
The reply is `{result, logs, calls, acting, ms}` or, when the program failed, `{error, …}`; `acting` lists every guarded call with its params and its result or error.
With `check` it is parsed and compiled only.
Closing the connection cancels the calls in flight; `machine` runs it there, over the held link.
