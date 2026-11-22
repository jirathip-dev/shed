wire: screen.watch
params: WatchParams
stream: ScreenUpdate

Stream a running terminal's screen.
After the response the server writes screen updates (see Screen updates), at most 20 a second: the full screen first (none when `since` is the screen's current `seq`), then the rows that changed.
It closes the connection when the program's PTY closes.
The connection still takes requests; their responses are not written.
