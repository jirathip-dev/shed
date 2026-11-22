`shed --on <machine> …` / `shed machine add|ls|rm`

The same commands on another host over SSH (key login only).
The server holds a link to each machine, so its terminals also show as `<machine>/<name>` in `ls` and `events` and take `get`, `read`, `send`, `keys`, `kill`, `wait`, `approve` and `cancel`, and `--on <machine> run --acp` starts an agent installed there; `machine ls --json` shows each link.
