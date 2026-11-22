wire: machine.stats
params: StatsParams
result: MachineStats

The machine's health now: load per CPU, memory pressure (percent of memory not available), swap used, free space on the volume of the state file, CPUs, uptime and the terminals by state (the reported state while running, else `running`; `exited`).
The server keeps one sample a minute for 24 h; `since` (unix seconds) adds those after it as `history`.
