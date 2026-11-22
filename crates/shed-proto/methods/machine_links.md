wire: machine.links
result: BTreeMap<String, Link>

Each configured machine's held link: `up`, `reconnecting` or `down`, since when, why it is down, and `down_since`, when it last left `up` (or was first tried; a retry does not move it); once up, the shed there (`hello`) and the machine's `os`, `arch` and `home`.
A request naming `<machine>/<name>` (`get`, `read`, `send`, `keys`, `kill`, `rm`, `state`, `acp.model`, `terminal.procs`, `wait`; `run` with `machine`, ACP ones bridged by that server) goes over the link; `ls` and `events` include every machine, their names so prefixed.
Reads the config again, linking machines added since.
