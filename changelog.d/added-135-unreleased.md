- `shed run --mem-max <size>` (or `$SHED_MEM_MAX` on `shed serve`, off by default) limits a terminal's process
  tree. On Linux with a user systemd that has the memory controller the program runs in its own scope with
  `MemoryMax`; elsewhere, two samples over ends the largest process the agent started, never the agent. Each
  is a `mem-limit` event and a note to the parent; `get`, `ls --json` and `top` show the limit, and
  `shed doctor` says which mechanism the host uses.
