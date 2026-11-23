- `machine.stats` reports macOS's memory pressure level as `mem_level` (`normal`, `warn` or `critical`), now and in
  `history`, and every process row in it and in `terminal.procs` carries `cpu_ms`, its CPU time, so two reads give a
  CPU share.
