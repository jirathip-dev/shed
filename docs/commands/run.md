`shed run [--name N] [--label k=v…] [--cwd D] [--keep] [--size RxC] [--mem-max SIZE] -- <cmd…>`

Start a terminal.
`--keep`: a lost program is replaced by a shell.
`--size`: rows by columns, default `$SHED_SIZE` of `shed serve`, else 50x200.
`--mem-max` (`200M`, `4G`; default `$SHED_MEM_MAX` of `shed serve`, else none): on Linux with a user systemd that has the memory controller the program gets its own scope with `MemoryMax`; elsewhere the sampler ends the largest process the agent started after two samples over, a guard against sustained use that a fast allocation can pass between samples.
Either is a `mem-limit` event; `shed doctor` says which.
