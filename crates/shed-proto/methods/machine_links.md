wire: machine.links
result: BTreeMap<String, Link>

Each configured machine's held link: `up`, `reconnecting` or `down`, since when, and why it is down.
A request naming `<machine>/<name>` (`get`, `read`, `send`, `keys`, `kill`, `rm`, `state`, `acp.model`, `wait`; `run` with `machine`, ACP ones bridged by that server) goes over the link; `ls` and `events` include every machine, their names so prefixed.
Reads the config again, linking machines added since.
