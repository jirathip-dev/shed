wire: machine.stats
params: StatsParams
result: MachineStats
on: machine

The machine's health now: load per CPU, memory pressure (percent of memory not available), free and total memory, swap used and pages swapped out since boot, free and total space on the volume of the state file, CPUs, uptime and the terminals by state (the reported state while running, else `running`; `exited`).
The server keeps one sample a minute for 24 h; `since` (unix seconds) adds those after it as `history`.
`top` adds that many processes using the most CPU (`top_cpu`) and the most memory (`top_mem`), each naming the terminal whose tree holds it; `machine` asks that machine, over its held link.
