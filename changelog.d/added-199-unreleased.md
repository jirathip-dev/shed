- `shed-client`, the one client for shed's socket: the handshake, a pool of idle connections with a call
  timeout, the typed calls and the streams, blocking and with no async runtime. shed's own CLI, machine
  links and ACP bridge run on it. A server shed cannot speak to is named in one `SHED_VERSION` line that
  says which side to update.
