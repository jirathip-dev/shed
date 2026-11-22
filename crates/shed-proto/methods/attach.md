wire: attach
params: Target
result: TerminalInfo
input: yes

Take over the terminal.
After the response the server writes a redraw (clear, then the screen text) and the program's raw output, and closes the connection when the program's PTY closes.
The client sends only notifications from then on (`terminal.input`, `keys`, `resize`); detaching is closing the connection.
