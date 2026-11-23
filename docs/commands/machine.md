`shed --on <machine> …` / `shed machine add|ls|rm|drain`

The same commands on another host over SSH (key login only).
The server holds a link to each machine, so its terminals also show as `<machine>/<name>` in `ls` and `events` and take `get`, `read`, `send`, `keys`, `kill`, `wait`, `approve` and `cancel`, and `--on <machine> run --acp` starts an agent installed there; `machine ls --json` shows each link.
`shed machine drain [--on <machine>] [--why TEXT]` drains that shed (default this one), as before a reboot or before its VM ends: `run` is refused with `DRAINING`, its warm spares end, and everything on a terminal that exists still works; `machine stats` shows `draining since <unix seconds> (<why>)`. It outlives a restart; draining again keeps the first time and reason, and `--off` lifts it.
