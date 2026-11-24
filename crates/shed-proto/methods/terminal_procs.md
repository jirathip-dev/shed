wire: terminal.procs
params: Target
result: Vec<Proc>
on: name

The terminal's live process tree, parents first, each with its `depth` below the terminal's own process, its CPU (percent of one core over its life, and `cpu_ms`, its CPU time, whose difference over two reads is a share), memory, age and command line; `own` marks the agent's own processes, not work it started.
A terminal that is not running has none.
