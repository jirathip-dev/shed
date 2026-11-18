- `shed top` and `shed ls` show an ACP terminal's agent state (`working`, `done`, `hibernated`, …) as STATE.
  `top` splits an ACP terminal's memory into AGENT (the bridge, adapter and agent as the turn began) and WORK
  (what it started), kept as `resources.agent_mb` and `work_mb`, and ends with a totals line. `shed top <t>
  [--full]` lists that terminal's processes over 50 MB: pid, memory, CPU, age and the command's name (the
  whole command line with `--full`).
