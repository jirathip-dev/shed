- The machine, file and process APIs an orchestrator needs to run work on other machines through shed:
  `file.write` takes a `mode` (set before any byte is written), `exec` an `env` (kept in memory only);
  `machine.links` reports each link's `down_since`, the shed there (`hello`) and the machine's `os`, `arch` and
  `home`; `machine.stats` adds free and total memory, pages swapped out, the volume's size, `top` (the
  processes using the most CPU and memory, each with its terminal) and `machine` (ask over the held link);
  `terminal.procs` gives a terminal's process tree (`shed top <terminal>` now uses it, also with `--on`);
  `machine.add` and `machine.rm` do what `shed machine add|rm` do, and adding the same pair twice is one machine.
