wire: terminal.readonly
params: TerminalParams

Mark the terminal read-only for this connection: from then on, until it closes, the connection's `send`, `keys`, `resize`, `attach`, `terminal.input`, `terminal.keys` and `terminal.resize` to the terminal are refused with `READONLY`.
The connection may resize only the terminals it has not marked.
