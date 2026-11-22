wire: run
params: RunParams
result: TerminalInfo
on: machine

Start a terminal.
`name` defaults to the first free `tN`; `keep` brings it back as a shell in its cwd after a server restart; `rows` and `cols` default to `$SHED_SIZE` (`RxC`) of the server, else 50x200.
ACP: `mcp_servers` (stdio, `http`, `sse`, in ACP's shapes) go to the agent; their env and header values are never saved or shown.
`meta` (an object) goes unchanged as the session's `_meta`.
`no_hibernate` keeps the agent running when idle.
`mem_max_mb` (default the server's `$SHED_MEM_MAX`, e.g. `4G`) limits the process tree's memory, and a `mem-limit` event (with `notify`, a `[shed] <name> mem-limit …` line to the parent) says what was ended: on Linux with a user systemd that has the memory controller the program runs in its own scope with `MemoryMax` and the kernel ends a process (the agent's children are ranked first); elsewhere, once the tree is over for two samples in a row, the sampler ends the largest process the agent started (SIGTERM, SIGKILL 2 s later), and an agent over it by itself is only reported.
The sampler is a guard against sustained use, not a hard limit: a fast allocation can pass it between samples.
`warm` takes the ready spare session of the same recipe (agent, command, cwd, policy, MCP servers, meta, size), renamed to `name` with these labels, and starts the next spare; spares are listed as `.warm-N`, at most `$SHED_WARM_MAX` (default 2) of them, each killed after `$SHED_WARM_IDLE` seconds (default 600) unused.
`parent` links it to the terminal that started it: when the parent's program ends the child is killed too, unless `outlive`; with `notify` a `[shed] <name> blocked|error|exited …` line is typed into the parent (a prompt for an ACP parent) when the child blocks, errors or ends, once per piece of news, held while the parent reports `working`.
