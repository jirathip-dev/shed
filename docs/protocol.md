# shed protocol

<!-- Generated from crates/shed-proto by `SHED_BLESS=1 cargo test -p shed-proto`; do not edit. -->

Version: `0.0.0`. Client and server must run the same version.

Newline-delimited [JSON-RPC 2.0](https://www.jsonrpc.org/specification) over a Unix socket, mode 0600:
`$XDG_RUNTIME_DIR/shed.sock` (macOS: `~/Library/Caches/shed/shed.sock`). One request per line, one
response per line; a request without an `id` is a notification and gets no response.

Every connection starts with `hello`. Any other method first is refused with `NO_HELLO`; another
product or version is refused with `SHED_VERSION: client shed x, server shed y; restart the server`
(a peer that sends no `product` shows as `unnamed`).

A terminal, as `run`, `ls`, `get`, `kill` and `rm` return it: `state` is `running` or `exited`; `exit` is
the exit status (128 + N for signal N), `null` while running or when the program was lost in a server
restart; `pid` is set while running; `reported` is the state the program last reported (absent until
then), or `stuck` while the program has left input unread for over a second (`send`, `keys` and
`acp.model` then fail, their input queued); `idle_after` is set when the `idle` fallback is on;
`resources` is the process tree's memory (MiB) and CPU (percent of one core) at the last sample, while
running; an ACP terminal's `session` is its agent session (`id`, for `run`'s `resume`; the current `model`
and the `models`, `[id, name]`; `run`'s `mcp_servers` with each env and header value as `***`; the agent's
slash `commands`, `[name, description, hint]`).

## Methods

### `hello`

The handshake; must come first on every connection. Another product or version is refused with `SHED_VERSION`.

```json
{"jsonrpc":"2.0","id":1,"method":"hello","params":{"product":"shed","version":"0.0.0"}}
{"jsonrpc":"2.0","id":1,"result":{"product":"shed","version":"0.0.0"}}
```

### `run`

Start a terminal. `name` defaults to the first free `tN`; `keep` brings it back as a shell in its cwd after a server restart; `rows` and `cols` default to `$SHED_SIZE` (`RxC`) of the server, else 50x200. ACP: `mcp_servers` (stdio, `http`, `sse`, in ACP's shapes) go to the agent; their env and header values are never saved or shown. `no_hibernate` keeps the agent running when idle.

```json
{"jsonrpc":"2.0","id":1,"method":"run","params":{"cmd":["make","test"],"cwd":"/home/user/project","keep":true,"labels":{"issue":"7","role":"impl"},"name":"build"}}
{"jsonrpc":"2.0","id":1,"result":{"cmd":["make","test"],"cwd":"/home/user/project","exit":null,"keep":true,"labels":{"issue":"7","role":"impl"},"name":"build","pid":4242,"resources":{"cpu_pct":3,"rss_mb":52},"state":"running"}}
```

### `ls`

List terminals, by name; `labels` keeps those that carry every given label.

```json
{"jsonrpc":"2.0","id":1,"method":"ls","params":{"labels":{"issue":"7"}}}
{"jsonrpc":"2.0","id":1,"result":[{"cmd":["make","test"],"cwd":"/home/user/project","exit":null,"keep":true,"labels":{"issue":"7","role":"impl"},"name":"build","pid":4242,"resources":{"cpu_pct":3,"rss_mb":52},"state":"running"}]}
```

### `get`

One terminal.

```json
{"jsonrpc":"2.0","id":1,"method":"get","params":{"name":"build"}}
{"jsonrpc":"2.0","id":1,"result":{"cmd":["make","test"],"cwd":"/home/user/project","exit":null,"keep":true,"labels":{"issue":"7","role":"impl"},"name":"build","pid":4242,"resources":{"cpu_pct":3,"rss_mb":52},"state":"running"}}
```

### `read`

The visible screen, or with `history` the scrollback and screen, as plain text; `lines` keeps the last N lines. ACP: the transcript as text, or with `events` its events (each with its `seq`), paged by `before` and `n`.

```json
{"jsonrpc":"2.0","id":1,"method":"read","params":{"history":false,"lines":2,"name":"build"}}
{"jsonrpc":"2.0","id":1,"result":{"text":"ok 12 tests\n$"}}
```

### `send`

Type text into the terminal.

```json
{"jsonrpc":"2.0","id":1,"method":"send","params":{"name":"build","text":"make lint\n"}}
{"jsonrpc":"2.0","id":1,"result":{}}
```

### `keys`

Press keys by name (`enter`, `esc`, `ctrl+c`, `up`, …); nothing is sent if any name is unknown.

```json
{"jsonrpc":"2.0","id":1,"method":"keys","params":{"keys":["ctrl+c","enter"],"name":"build"}}
{"jsonrpc":"2.0","id":1,"result":{}}
```

### `kill`

End the terminal's whole process tree (SIGTERM, then SIGKILL); returns the exited terminal.

```json
{"jsonrpc":"2.0","id":1,"method":"kill","params":{"name":"build"}}
{"jsonrpc":"2.0","id":1,"result":{"cmd":["make","test"],"cwd":"/home/user/project","exit":0,"keep":true,"labels":{"issue":"7","role":"impl"},"name":"build","pid":null,"state":"exited"}}
```

### `rm`

Remove an exited terminal; returns it as it was.

```json
{"jsonrpc":"2.0","id":1,"method":"rm","params":{"name":"build"}}
{"jsonrpc":"2.0","id":1,"result":{"cmd":["make","test"],"cwd":"/home/user/project","exit":0,"keep":true,"labels":{"issue":"7","role":"impl"},"name":"build","pid":null,"state":"exited"}}
```

### `resize`

Resize the terminal; the program gets SIGWINCH.

```json
{"jsonrpc":"2.0","id":1,"method":"resize","params":{"cols":120,"name":"build","rows":50}}
{"jsonrpc":"2.0","id":1,"result":{}}
```

### `attach`

Take over the terminal. After the response the server writes a redraw (clear, then the screen text) and the program's raw output, and closes the connection when the program's PTY closes. The client sends only notifications from then on (`send`, `keys`, `resize`); detaching is closing the connection.

```json
{"jsonrpc":"2.0","id":1,"method":"attach","params":{"name":"build"}}
{"jsonrpc":"2.0","id":1,"result":{"cmd":["make","test"],"cwd":"/home/user/project","exit":null,"keep":true,"labels":{"issue":"7","role":"impl"},"name":"build","pid":4242,"resources":{"cpu_pct":3,"rss_mb":52},"state":"running"}}
```

### `state`

Report the terminal's state (`working`, `blocked`, `done` or `error`) with an optional note; a program can also print `ESC ] 7501 ; state=<s>[;note=<n>] BEL`, which never reaches the screen.

```json
{"jsonrpc":"2.0","id":1,"method":"state","params":{"name":"build","note":"tests pass","state":"done"}}
{"jsonrpc":"2.0","id":1,"result":{"cmd":["make","test"],"cwd":"/home/user/project","exit":null,"keep":true,"labels":{"issue":"7","role":"impl"},"name":"build","pid":4242,"reported":{"note":"tests pass","state":"done"},"resources":{"cpu_pct":3,"rss_mb":52},"state":"running"}}
```

### `wait`

Answer once the reported state is one of `until` (`idle` too, from the `idle_after` fallback) or a visible screen line matches the pattern `output` (`matched`), the program exits (`exited`) or `timeout` seconds pass (`timeout`). The server blocks; nothing polls.

```json
{"jsonrpc":"2.0","id":1,"method":"wait","params":{"name":"build","output":"^ok \\d+ tests$","timeout":600.0,"until":["done","blocked"]}}
{"jsonrpc":"2.0","id":1,"result":{"outcome":"matched","terminal":{"cmd":["make","test"],"cwd":"/home/user/project","exit":null,"keep":true,"labels":{"issue":"7","role":"impl"},"name":"build","pid":4242,"reported":{"note":"tests pass","state":"done"},"resources":{"cpu_pct":3,"rss_mb":52},"state":"running"}}}
```

### `events`

Stream events. After the response the server writes one event per line (see Events) for the terminals carrying every one of `labels`, first those after `since` still in the ring of the last 1,000, and never closes the connection; closing it unsubscribes.

```json
{"jsonrpc":"2.0","id":1,"method":"events","params":{"labels":{"issue":"7"},"since":120}}
{"jsonrpc":"2.0","id":1,"result":{}}
```

### `emit`

Emit one of an ACP terminal's structured events (`turn`, `text`, `tool_call`, `plan`, `mode`, `approval_request`, `approved`, `reply`); its bridge (`shed run --acp`) calls this. Other events are refused.

```json
{"jsonrpc":"2.0","id":1,"method":"emit","params":{"event":"reply","name":"agent","stop_reason":"end_turn","text":"All 12 tests pass."}}
{"jsonrpc":"2.0","id":1,"result":{}}
```

### `screen.watch`

Stream a running terminal's screen. After the response the server writes screen updates (see Screen updates), at most 20 a second: the full screen first (none when `since` is the screen's current `seq`), then the rows that changed. It closes the connection when the program's PTY closes. The connection still takes requests; their responses are not written.

```json
{"jsonrpc":"2.0","id":1,"method":"screen.watch","params":{"since":41,"terminal":"build"}}
{"jsonrpc":"2.0","id":1,"result":{}}
```

### `terminal.input`

Type text into the terminal, as `send`.

```json
{"jsonrpc":"2.0","id":1,"method":"terminal.input","params":{"terminal":"build","text":"make lint"}}
{"jsonrpc":"2.0","id":1,"result":{}}
```

### `terminal.keys`

Press keys by name, as `keys`; `ctrl-x` is `ctrl+x`.

```json
{"jsonrpc":"2.0","id":1,"method":"terminal.keys","params":{"keys":["esc","ctrl-c","up","enter"],"terminal":"build"}}
{"jsonrpc":"2.0","id":1,"result":{}}
```

### `terminal.resize`

Resize the terminal, as `resize`; without `terminal`, the one this connection watches.

```json
{"jsonrpc":"2.0","id":1,"method":"terminal.resize","params":{"cols":60,"rows":30}}
{"jsonrpc":"2.0","id":1,"result":{}}
```

### `terminal.readonly`

Mark the terminal read-only for this connection: from then on, until it closes, the connection's `send`, `keys`, `resize`, `attach`, `terminal.input`, `terminal.keys` and `terminal.resize` to the terminal are refused with `READONLY`. The connection may resize only the terminals it has not marked.

```json
{"jsonrpc":"2.0","id":1,"method":"terminal.readonly","params":{"terminal":"agent"}}
{"jsonrpc":"2.0","id":1,"result":{}}
```

### `machine.stats`

The machine's health now: load per CPU, memory pressure (percent of memory not available), swap used, free space on the volume of the state file, CPUs, uptime and the terminals by state (the reported state while running, else `running`; `exited`). The server keeps one sample a minute for 24 h; `since` (unix seconds) adds those after it as `history`.

```json
{"jsonrpc":"2.0","id":1,"method":"machine.stats","params":{"since":1762416000}}
{"jsonrpc":"2.0","id":1,"result":{"at":1762416090,"cpus":8,"disk_free_mb":120000,"history":[{"at":1762416060,"cpus":8,"disk_free_mb":120000,"load_per_cpu":0.25,"mem_pressure_pct":42,"swap_used_mb":512,"terminals":{"exited":1,"working":2},"uptime_s":86400}],"load_per_cpu":0.25,"mem_pressure_pct":42,"swap_used_mb":512,"terminals":{"exited":1,"working":2},"uptime_s":86400}}
```

### `acp.model`

Switch an ACP terminal's model to one of its `session.models`: its bridge asks the agent (`session/set_config_option`, the `model` option) and emits `session` with the new `model`.

```json
{"jsonrpc":"2.0","id":1,"method":"acp.model","params":{"model":"haiku","name":"agent"}}
{"jsonrpc":"2.0","id":1,"result":{}}
```

### `machine.links`

Each configured machine's held link: `up`, `reconnecting` or `down`, since when, and why it is down. A request naming `<machine>/<name>` (`get`, `read`, `send`, `keys`, `kill`, `rm`, `state`, `acp.model`, `wait`; `run` with `machine`, ACP ones bridged by that server) goes over the link; `ls` and `events` include every machine, their names so prefixed. Reads the config again, linking machines added since.

```json
{"jsonrpc":"2.0","id":1,"method":"machine.links","params":{}}
{"jsonrpc":"2.0","id":1,"result":{"box":{"since":1762416000,"state":"up"},"old":{"error":"SHED_VERSION: shed 0.1.0 here, 0.2.0 on old; install the same version on both","since":1762416000,"state":"down"}}}
```

### `exec`

Run a program without a terminal in a process group of its own, `stdin` fed to it, and answer with its exit code (128 + the signal when one ended it) and the exact bytes it wrote. The group is ended when the program returns or `timeout` seconds pass (`IO`); output over `REPLY_LIMIT` bytes is `IO`, never cut. `machine` runs it there, over the held link.

```json
{"jsonrpc":"2.0","id":1,"method":"exec","params":{"cmd":["git","status","--short"],"cwd":"/home/user/project","machine":"box","timeout":30.0}}
{"jsonrpc":"2.0","id":1,"result":{"code":0,"stderr":[],"stdout":[32,77,32,115,114,99,47,108,105,98,46,114,115,10]}}
```

### `file.read`

A file's bytes; one over `REPLY_LIMIT` bytes is `IO`. `machine` reads it there.

```json
{"jsonrpc":"2.0","id":1,"method":"file.read","params":{"path":"/home/user/project/VERSION"}}
{"jsonrpc":"2.0","id":1,"result":[49,46,50,46,48,10]}
```

### `file.write`

Replace a file with `data`: written to a temp file beside it, then renamed over it, its mode kept. `machine` writes it there.

```json
{"jsonrpc":"2.0","id":1,"method":"file.write","params":{"data":[49,46,51,46,48,10],"path":"/home/user/project/VERSION"}}
{"jsonrpc":"2.0","id":1,"result":{}}
```

### `rpc.discover`

The whole API as one OpenRPC document, as `shed schema` prints it (`docs/shed.openrpc.json`): every method's params and result, the streamed events and screen updates, and the error codes, as JSON Schema. Needs no `hello`.

```json
{"jsonrpc":"2.0","id":1,"method":"rpc.discover","params":{}}
{"jsonrpc":"2.0","id":1,"result":{"info":{"title":"shed","version":"0.0.0"},"openrpc":"1.3.2"}}
```

## Events

An event names its terminal, carries its labels and counts up in `seq`; `event` is `started`, `state` (the reported state changed), `exited` (by itself, with its `code`), `killed` (by `kill`) or `resources` (memory or CPU moved by more than 10 % since the last one).

An ACP terminal (`shed run --acp`) adds its agent's events, emitted by its bridge: `turn` (a prompt), `text` (a reply chunk), `tool_call` (started or changed), `plan`, `mode`, `approval_request` (answer with `shed approve`), `approved`, `reply` (the turn's whole reply and its `stop_reason`) and `session` (the session is ready, or its model or slash commands changed). A resumed session's history (`run` with `resume`) comes first, its events marked `"replay": true` so a client does not count them as new. An idle agent hibernates (state `hibernated`, its processes stopped); the next send wakes it, its history replayed the same way.

```json
{"seq":1,"name":"build","labels":{"issue":"7","role":"impl"},"event":"started"}
{"seq":2,"name":"build","labels":{"issue":"7","role":"impl"},"event":"state","state":"done","note":"tests pass"}
{"seq":3,"name":"build","labels":{"issue":"7","role":"impl"},"event":"resources","rss_mb":52,"cpu_pct":3}
{"seq":4,"name":"build","labels":{"issue":"7","role":"impl"},"event":"exited","code":0}
{"seq":5,"name":"build","labels":{"issue":"7","role":"impl"},"event":"killed"}
{"seq":6,"name":"build","labels":{"issue":"7","role":"impl"},"event":"turn","prompt":"Fix the failing test"}
{"seq":7,"name":"build","labels":{"issue":"7","role":"impl"},"event":"text","text":"Looking at the test"}
{"seq":8,"name":"build","labels":{"issue":"7","role":"impl"},"event":"tool_call","id":"call_1","title":"Edit src/lib.rs","status":"pending","kind":"edit"}
{"seq":9,"name":"build","labels":{"issue":"7","role":"impl"},"event":"plan","entries":[{"content":"Fix the test","priority":"high","status":"in_progress"}]}
{"seq":10,"name":"build","labels":{"issue":"7","role":"impl"},"event":"mode","mode":"default"}
{"seq":11,"name":"build","labels":{"issue":"7","role":"impl"},"event":"approval_request","tool":"Edit src/lib.rs","options":[{"id":"allow","name":"Allow","kind":"allow_once"},{"id":"reject","name":"Reject","kind":"reject_once"}]}
{"seq":12,"name":"build","labels":{"issue":"7","role":"impl"},"event":"approved","option":"allow"}
{"seq":13,"name":"build","labels":{"issue":"7","role":"impl"},"event":"reply","text":"Fixed; all 12 tests pass.","stop_reason":"end_turn"}
{"seq":14,"name":"build","labels":{"issue":"7","role":"impl"},"event":"session","id":"sess_1","model":"opus","models":[["opus","Opus"],["haiku","Haiku"]],"mcp_servers":[{"headers":[{"name":"Authorization","value":"***"}],"name":"web","type":"http","url":"https://mcp.example/"}],"commands":[["review","Review the diff","files"]]}
```

## Screen updates

`screen.watch` writes one update per line. `seq` is the screen's change count; `rows`, `cols`, `cursor` (`[row, col]`, from 0) and `scrollback` (lines) are always there; `lines` holds the rows that changed, each as runs of cells with one pen: `fg` and `bg` (a palette index 0–255, or `[r, g, b]`; `null` is the default), `bold`, `italic`, `underline` and `inverse`. Trailing blank default cells are dropped, so a blank row has no runs. `full` marks an update that carries every row: the first, and the one after a resize.

```json
{"seq":41,"full":true,"rows":2,"cols":40,"cursor":[1,2],"scrollback":120,"lines":[{"row":0,"runs":[{"text":"ok","fg":2,"bg":null,"bold":true,"italic":false,"underline":false,"inverse":false},{"text":" 12 tests","fg":null,"bg":null,"bold":false,"italic":false,"underline":false,"inverse":false}]},{"row":1,"runs":[{"text":"$","fg":null,"bg":null,"bold":false,"italic":false,"underline":false,"inverse":false}]}]}
{"seq":44,"full":false,"rows":2,"cols":40,"cursor":[1,6],"scrollback":120,"lines":[{"row":1,"runs":[{"text":"$ ","fg":null,"bg":null,"bold":false,"italic":false,"underline":false,"inverse":false},{"text":"make","fg":null,"bg":[40,40,40],"bold":false,"italic":false,"underline":false,"inverse":false}]}]}
```

## Errors

An error's `message` starts with its name: `NAME: detail`.

| Code | Name |
|---|---|
| -32700 | `PARSE` |
| -32600 | `INVALID_REQUEST` |
| -32601 | `NO_SUCH_METHOD` |
| -32602 | `INVALID_PARAMS` |
| -32603 | `INTERNAL` |
| -32000 | `SHED_VERSION` |
| -32001 | `NO_HELLO` |
| -32002 | `NO_SUCH_TERMINAL` |
| -32003 | `NAME_TAKEN` |
| -32004 | `STILL_RUNNING` |
| -32005 | `SPAWN_FAILED` |
| -32006 | `READONLY` |
| -32007 | `NO_LINK` |
| -32008 | `IO` |

```json
{"jsonrpc":"2.0","id":1,"error":{"code":-32000,"message":"SHED_VERSION: client 0.1.0, server 0.0.0; restart the server"}}
```
