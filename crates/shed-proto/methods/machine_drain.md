wire: machine.drain
params: DrainParams
on: machine

Drain this shed, as `shed machine drain` does: from then on `run` is refused with `DRAINING` and its warm spares end, while every call on a terminal that exists still works; `machine.stats` shows `draining`. It is kept in `drain.json` beside the state file, so it outlives a restart. Draining twice keeps the first time and reason; `on: false` lifts it; `machine` asks that machine, over its held link.
