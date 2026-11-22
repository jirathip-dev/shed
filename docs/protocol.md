# shed protocol

<!-- Generated from crates/shed-proto by `SHED_BLESS=1 cargo test -p shed-proto --features schema`; do not edit. -->

Version: `0.0.0`, protocol `5`, oldest served `1`.

Newline-delimited [JSON-RPC 2.0](https://www.jsonrpc.org/specification) over a Unix socket, mode 0600:
`$XDG_RUNTIME_DIR/shed.sock` (macOS: `~/Library/Caches/shed/shed.sock`). One request per line, one
response per line; a request without an `id` is a notification and gets no response.

Every connection starts with `hello`. Any other method first is refused with `NO_HELLO`. Each side
sends its `protocol` and the `oldest` it still speaks: a server the oldest protocol it serves, a client
the oldest server protocol it accepts. A server on protocol N serves N and N-1 for one release window,
and answers an N-1 client in N-1's shapes: its `hello` result carries `protocol` N-1 and no `oldest`.
Another product, or ranges that don't overlap, are refused with `SHED_VERSION: client <sha> protocol
<a>-<b> / server <sha> protocol <c>-<d>; update <side>` (a peer that sends no `product` shows as
`unnamed`, no `protocol` as 0, no `oldest` as its `protocol`).

A terminal, as `run`, `ls`, `get`, `kill` and `rm` return it: `state` is `running` or `exited`; `exit` is the
exit status (128 + N for signal N), `null` while running or when the program was lost in a server restart;
`pid` is set while running; `reported` is the state the program last reported (absent until then), with OSC
7501's `kind`, `progress`, `app`, `title` and `msg` when it sent them, or `stuck` while the program has left
input unread for over a second (`send`, `keys` and `acp.model` then fail, their input queued); `records` are
its OSC 7501 reports with an `id`, one per id; `idle_after` is set when the `idle` fallback is on;
`resources` is the process tree's memory (MiB) and CPU (percent of one core) at the last sample, while
running; an ACP terminal's `session` is its agent session (`id`, for `run`'s `resume`; the current `model`
and the `models`, `[id, name]`; `run`'s `mcp_servers` with each env and header value as `***`; the agent's
slash `commands`, `[name, description, hint]`; its last `usage`: `used` and `size` of the context window in
tokens, and the `cost` when sent); `parent` names the terminal that started it (absent without one), with
`outlive` and `notify` as `run` set them; `redacted` (a login's terminal) means `read`, `screen.watch`,
`wait`'s `output` and its events show its text with anything like a token as `[redacted]`, and `attach` is refused;
`account` is the account it runs on.

## Methods

### `account.add`

Make an account: an empty config folder of mode 0700 under `accounts/<provider>/<name>`, or with `folder` a link to that existing folder, adopted in place; a folder holding one of the record's `setup_files` is refused (`ACCOUNT_SETUP`). `token_file`, a file of mode 0600 on this host, is copied in as the account's setup token. The same call twice is one account.

```json
{"jsonrpc":"2.0","id":1,"method":"account.add","params":{"name":"work","provider":"claude","token_file":"/home/user/claude-work.token"}}
{"jsonrpc":"2.0","id":1,"result":{"adopted":false,"checked_at":1762416000,"folder":"/home/user/.local/state/shed/accounts/claude/work","line":"signed in · claude.ai (dev@example.com)","name":"work","provider":"claude","runs":1,"setup":[],"state":"ready","token":{"expires_at":1793536000,"rejected":false,"saved_at":1762000000},"usage":{"read_at":1762416000,"windows":[{"id":"5h","resets_at":1762430400,"used_pct":12}]},"who":"dev@example.com"}}
```

### `account.login`

The record's `login` in a redacted terminal `login-<provider>-<name>` with the account's folder set; a second call while it runs returns the same terminal. shed kills it after 600 s; when it ends shed runs `account.status`.

```json
{"jsonrpc":"2.0","id":1,"method":"account.login","params":{"name":"work","provider":"claude"}}
{"jsonrpc":"2.0","id":1,"result":{"terminal":"login-claude-work"}}
```

### `account.ls`

The accounts, per provider `default` first, then in the order `auto` picks: what shed last read of each, its token's dates (never the token), the setup files its folder holds and the terminals on it. A reading older than the record's `usage.fresh` is read again in the background.

```json
{"jsonrpc":"2.0","id":1,"method":"account.ls","params":{"provider":"claude"}}
{"jsonrpc":"2.0","id":1,"result":[{"adopted":false,"checked_at":1762416000,"folder":"/home/user/.local/state/shed/accounts/claude/work","line":"signed in · claude.ai (dev@example.com)","name":"work","provider":"claude","runs":1,"setup":[],"state":"ready","token":{"expires_at":1793536000,"rejected":false,"saved_at":1762000000},"usage":{"read_at":1762416000,"windows":[{"id":"5h","resets_at":1762430400,"used_pct":12}]},"who":"dev@example.com"}]}
```

### `account.rm`

The record's `logout`, then the account's folder (an adopted one: only its link), its token and its readings; refused for `default` and while its login runs.

```json
{"jsonrpc":"2.0","id":1,"method":"account.rm","params":{"name":"work","provider":"claude"}}
{"jsonrpc":"2.0","id":1,"result":{}}
```

### `account.status`

Run the record's status command and usage probe for the account now, and keep what they say.

```json
{"jsonrpc":"2.0","id":1,"method":"account.status","params":{"name":"work","provider":"claude"}}
{"jsonrpc":"2.0","id":1,"result":{"adopted":false,"checked_at":1762416000,"folder":"/home/user/.local/state/shed/accounts/claude/work","line":"signed in · claude.ai (dev@example.com)","name":"work","provider":"claude","runs":1,"setup":[],"state":"ready","token":{"expires_at":1793536000,"rejected":false,"saved_at":1762000000},"usage":{"read_at":1762416000,"windows":[{"id":"5h","resets_at":1762430400,"used_pct":12}]},"who":"dev@example.com"}}
```

### `acp.model`

Switch an ACP terminal's model to one of its `session.models`: its bridge asks the agent (`session/set_config_option`, the `model` option) and emits `session` with the new `model`.

```json
{"jsonrpc":"2.0","id":1,"method":"acp.model","params":{"model":"haiku","name":"agent"}}
{"jsonrpc":"2.0","id":1,"result":{}}
```

### `agents`

Each agent shed has a provider record for or has installed, by id, as `shed agents --json` prints it: the record (its limit and login-expired lines, hooks and program facts), its `source` (`built-in` or the user file in `<config>/providers/` that adds or overrides it) and the installed adapter. A bad user file is skipped (`shed doctor` names it).

```json
{"jsonrpc":"2.0","id":1,"method":"agents","params":{}}
{"jsonrpc":"2.0","id":1,"result":[{"id":"fake-acp","installed":{"cmd":["npx","-y","fake-acp@0.1.0"],"distribution":"npx","id":"fake-acp","version":"0.1.0"},"medians":{"first_token_ms":1900,"ready_ms":2500,"sessions":20},"provider":{"agent":"fake","auth_failures":["fake: login gone"],"cache":{"api":300,"api_vars":["FAKE_API_KEY"],"plan":3600},"config_dir_var":"FAKE_HOME","driver":"acp","effort_args":["--effort","{effort}"],"efforts":["low","high"],"hooks":{"events":[["Stop","done"]],"file":".fake/settings.json"},"id":"fake-acp","install":"npm install -g fake-agent","limit":["fake: limit hit"],"login":"fake login","logout":"fake logout","model_args":["--model","{model}"],"model_env":null,"models":[["fast","Fast"]],"proxy":null,"quota":{"cache_read":"cachedInputTokens","cache_write":"cachedWriteTokens","input":"inputTokens","output":"outputTokens","path":["quota","token_count"]},"resets":["back at "],"rules":[{"any":["Trust this folder?"],"id":"trust","kind":"permission","priority":10,"region":20,"state":"blocked","version":"0.1.0"}],"scrub":[],"session_files":[],"settings_var":null,"setup_files":[],"status":"fake login status","status_read":null,"stream":null,"terminal":null,"token":null,"token_var":"FAKE_TOKEN","usage":null},"source":"/home/me/.config/shed/providers/fake.toml"}]}
```

### `agents.match`

Apply agent `id`'s limit and auth rules to `text` (a pane's screen, a reply): the first line that says a usage limit was hit with its reset time, and the first that says the login expired, as an ACP lane would read them. An agent with no record matches nothing.

```json
{"jsonrpc":"2.0","id":1,"method":"agents.match","params":{"id":"fake-acp","text":"working…\nfake: limit hit, back at 3pm."}}
{"jsonrpc":"2.0","id":1,"result":{"auth":null,"limit":"fake: limit hit, back at 3pm.","resets":"3pm"}}
```

### `attach`

Take over the terminal. After the response the server writes a redraw (clear, then the screen text) and the program's raw output, and closes the connection when the program's PTY closes. The client sends only notifications from then on (`terminal.input`, `keys`, `resize`); detaching is closing the connection.

```json
{"jsonrpc":"2.0","id":1,"method":"attach","params":{"name":"build"}}
{"jsonrpc":"2.0","id":1,"result":{"cmd":["make","test"],"cwd":"/home/user/project","exit":null,"keep":true,"labels":{"issue":"7","role":"impl"},"name":"build","parent":{"name":"lead","notify":true},"pid":4242,"resources":{"cpu_pct":3,"rss_mb":52},"state":"running"}}
```

### `code.find`

The files under `root` that best answer `query`'s words, best first: the top 10 (`top`, at most 50) of a stated `total`. A word earns points in a file's name, its folders, its symbols' names and its lines; the sum is multiplied by the words that earned any and halved for a test. Each file names its score, short `why` reasons and the symbols that earned them. The walk and its limits are `code.grep`'s. `machine` searches there.

```json
{"jsonrpc":"2.0","id":1,"method":"code.find","params":{"hidden":false,"query":"job retries","root":"/home/user/project","top":2}}
{"jsonrpc":"2.0","id":1,"result":{"files":[{"lang":"rust","lines":49,"path":"src/job.rs","score":136,"symbols":[{"depth":0,"end":14,"kind":"struct","name":"Job","start":9},{"depth":0,"end":49,"kind":"impl","name":"Job","start":37}],"why":["name is job","lines: job 9, retries 4"]}],"searched":12,"total":1}}
```

### `code.grep`

The lines under `root` that match, grouped by the symbols around them, in name order. The walk keeps to `.gitignore` and `info/exclude`, skips `.git`, dot names without `hidden`, symlinks and binary files. A line over 4,096 bytes comes back `clipped` with its `length`. Over 20,000 files, 250 rows, 256 MiB or 10 s is `LIMIT`. `machine` searches there.

```json
{"jsonrpc":"2.0","id":1,"method":"code.grep","params":{"count":false,"fixed":false,"glob":["*.rs"],"hidden":false,"ignore_case":false,"pattern":"retries","root":"/home/user/project","word":true}}
{"jsonrpc":"2.0","id":1,"result":{"files":[{"count":2,"groups":[{"lines":[{"n":13,"text":"    pub retries: usize,"}],"scope":[{"depth":0,"end":14,"kind":"struct","name":"Job","start":9}]},{"lines":[{"n":42,"text":"        let retries = self.retries.min(budget);"}],"scope":[{"depth":0,"end":49,"kind":"impl","name":"Job","start":37},{"depth":1,"end":44,"kind":"fn","name":"run","start":38}]}],"lang":"rust","path":"src/job.rs"}],"matches":2,"searched":12}}
```

### `code.outline`

A file's functions, types, sections and blocks in file order, each with its kind, name, lines and depth, by line rules for Rust, TypeScript/JavaScript, Python, Markdown and TOML and by indentation for other files. Over 2,000 symbols or 256 MiB is `LIMIT`. `machine` reads it there.

```json
{"jsonrpc":"2.0","id":1,"method":"code.outline","params":{"machine":"box","path":"/home/user/project/src/job.rs"}}
{"jsonrpc":"2.0","id":1,"result":{"lang":"rust","lines":49,"path":"/home/user/project/src/job.rs","symbols":[{"depth":0,"end":14,"kind":"struct","name":"Job","start":9},{"depth":0,"end":49,"kind":"impl","name":"Job","start":37},{"depth":1,"end":44,"kind":"fn","name":"run","start":38}]}}
```

### `emit`

Emit one of an ACP terminal's structured events (`turn`, `text`, `tool_call`, `plan`, `mode`, `approval_request`, `approved`, `reply`); its bridge (`shed run --acp`) calls this. Other events are refused.

```json
{"jsonrpc":"2.0","id":1,"method":"emit","params":{"event":"reply","name":"agent","stop_reason":"end_turn","text":"All 12 tests pass."}}
{"jsonrpc":"2.0","id":1,"result":{}}
```

### `events`

Stream events. After the response the server writes one event per line (see Events) for the terminals carrying every one of `labels`, first those after `since` still kept (the last 4,096 events other than `text`, and each terminal's last 256 `text` chunks), and never closes the connection; closing it unsubscribes. When events after `since` are gone it first writes `{"jsonrpc":"2.0","method":"reset","params":{"seq":N}}`: those up to `N` are lost. A client more than 4,096 events behind gets `{"jsonrpc":"2.0","method":"lagged","params":{"seq":N}}` and the connection is closed; it resumes with `since`. A connection that sent `heartbeat` gets `{"jsonrpc":"2.0","method":"heartbeat"}` lines when idle.

```json
{"jsonrpc":"2.0","id":1,"method":"events","params":{"labels":{"issue":"7"},"since":120}}
{"jsonrpc":"2.0","id":1,"result":{}}
```

### `exec`

Run a program without a terminal in a process group of its own, `stdin` fed to it and `env` over the server's variables (kept in memory only), and answer with its exit code (128 + the signal when one ended it) and the exact bytes it wrote. The group is ended when the program returns or `timeout` seconds pass (`IO`); output over `REPLY_LIMIT` bytes is `IO`, never cut. `machine` runs it there, over the held link.

```json
{"jsonrpc":"2.0","id":1,"method":"exec","params":{"cmd":["git","status","--short"],"cwd":"/home/user/project","env":{"GIT_PAGER":"cat"},"machine":"box","timeout":30.0}}
{"jsonrpc":"2.0","id":1,"result":{"code":0,"stderr":[],"stdout":[32,77,32,115,114,99,47,108,105,98,46,114,115,10]}}
```

### `file.read`

A file's bytes, or with `lines` only that span's; over `REPLY_LIMIT` bytes is `IO`. `machine` reads it there.

```json
{"jsonrpc":"2.0","id":1,"method":"file.read","params":{"lines":{"end":3,"start":3},"path":"/home/user/project/CHANGELOG.md"}}
{"jsonrpc":"2.0","id":1,"result":[35,35,32,49,46,50,46,48,10]}
```

### `file.write`

Replace a file with `data`: written to a temp file beside it, then renamed over it, its mode kept, or `mode` when given (the temp file has it before any byte is written). `machine` writes it there.

```json
{"jsonrpc":"2.0","id":1,"method":"file.write","params":{"data":[49,46,51,46,48,10],"mode":384,"path":"/home/user/project/VERSION"}}
{"jsonrpc":"2.0","id":1,"result":{}}
```

### `get`

One terminal.

```json
{"jsonrpc":"2.0","id":1,"method":"get","params":{"name":"build"}}
{"jsonrpc":"2.0","id":1,"result":{"cmd":["make","test"],"cwd":"/home/user/project","exit":null,"keep":true,"labels":{"issue":"7","role":"impl"},"name":"build","parent":{"name":"lead","notify":true},"pid":4242,"resources":{"cpu_pct":3,"rss_mb":52},"state":"running"}}
```

### `handover`

Hand over to the binary the server was started from, which must be `build`: after the same check as `handover.check` it answers, waits up to 10 s for the other requests in flight, then replaces itself in place on the same process id, keeping the socket and every running terminal (screen, children and exit statuses). Every connection is closed: a closed connection means reconnect and ask again; `events` with `since` gets `reset`. A refused or failed handover leaves the server serving as it was.

```json
{"jsonrpc":"2.0","id":1,"method":"handover","params":{"build":"1a2b3c4"}}
{"jsonrpc":"2.0","id":1,"result":{"from":"4254d78","terminals":12,"to":"1a2b3c4"}}
```

### `handover.check`

Would the server hand over to the binary at `candidate`: it runs `<candidate> serve --preflight`, which must name `build`, read the state file and inherit this server's handover format, and speak the protocol of every live ACP bridge. Changes nothing; `HANDOVER_REFUSED` says why not.

```json
{"jsonrpc":"2.0","id":1,"method":"handover.check","params":{"build":"1a2b3c4","candidate":"/home/user/.cargo/bin/shed.new"}}
{"jsonrpc":"2.0","id":1,"result":{"from":"4254d78","terminals":12,"to":"1a2b3c4"}}
```

### `heartbeat`

Keep the connection alive. Once a connection sends one, the server closes it when nothing comes from it for 30 s (two missed beats of 15 s), and writes a `heartbeat` line every 15 s into its idle `events` stream. A write the client does not take within 30 s also closes any connection.

```json
{"jsonrpc":"2.0","id":1,"method":"heartbeat","params":{}}
{"jsonrpc":"2.0","id":1,"result":{}}
```

### `hello`

The handshake; must come first on every connection. Another product, or a protocol range that does not overlap the server's, is refused with `SHED_VERSION`.

```json
{"jsonrpc":"2.0","id":1,"method":"hello","params":{"build":"4254d78","oldest":4,"product":"shed","protocol":5,"version":"0.0.0"}}
{"jsonrpc":"2.0","id":1,"result":{"build":"4254d78","oldest":4,"product":"shed","protocol":5,"version":"0.0.0"}}
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
{"jsonrpc":"2.0","id":1,"result":{"cmd":["make","test"],"cwd":"/home/user/project","exit":0,"keep":true,"labels":{"issue":"7","role":"impl"},"name":"build","parent":{"name":"lead","notify":true},"pid":null,"state":"exited"}}
```

### `ls`

List terminals, by name; `labels` keeps those that carry every given label, `parent` the children of that terminal.

```json
{"jsonrpc":"2.0","id":1,"method":"ls","params":{"labels":{"issue":"7"},"parent":"lead"}}
{"jsonrpc":"2.0","id":1,"result":[{"cmd":["make","test"],"cwd":"/home/user/project","exit":null,"keep":true,"labels":{"issue":"7","role":"impl"},"name":"build","parent":{"name":"lead","notify":true},"pid":4242,"resources":{"cpu_pct":3,"rss_mb":52},"state":"running"}]}
```

### `machine.add`

Add a machine to the config file, as `shed machine add` does, and start holding its link; the same name and target again is the same machine, another target under a taken name `INVALID_PARAMS`.

```json
{"jsonrpc":"2.0","id":1,"method":"machine.add","params":{"name":"box","target":"user@box.example"}}
{"jsonrpc":"2.0","id":1,"result":{}}
```

### `machine.links`

Each configured machine's held link: `up`, `reconnecting` or `down`, since when, why it is down, and `down_since`, when it last left `up` (or was first tried; a retry does not move it); once up, the shed there (`hello`) and the machine's `os`, `arch` and `home`. A request naming `<machine>/<name>` (`get`, `read`, `send`, `keys`, `kill`, `rm`, `state`, `acp.model`, `terminal.procs`, `wait`; `run` with `machine`, ACP ones bridged by that server) goes over the link; `ls` and `events` include every machine, their names so prefixed. Reads the config again, linking machines added since.

```json
{"jsonrpc":"2.0","id":1,"method":"machine.links","params":{}}
{"jsonrpc":"2.0","id":1,"result":{"box":{"arch":"x86_64","hello":{"build":"4254d78","oldest":1,"product":"shed","protocol":5,"version":"0.0.0"},"home":"/home/user","os":"linux","since":1762416000,"state":"up"},"old":{"down_since":1762415000,"error":"SHED_VERSION: shed 0.1.0 here, 0.2.0 on old; install the same version on both","since":1762416000,"state":"down"}}}
```

### `machine.rm`

Remove a machine from the config file, as `shed machine rm` does, and drop its held link; removing one that is not there is a no-op.

```json
{"jsonrpc":"2.0","id":1,"method":"machine.rm","params":{"name":"box"}}
{"jsonrpc":"2.0","id":1,"result":{}}
```

### `machine.stats`

The machine's health now: load per CPU, memory pressure (percent of memory not available), free and total memory, swap used and pages swapped out since boot, free and total space on the volume of the state file, CPUs, uptime and the terminals by state (the reported state while running, else `running`; `exited`). The server keeps one sample a minute for 24 h; `since` (unix seconds) adds those after it as `history`. `top` adds that many processes using the most CPU (`top_cpu`) and the most memory (`top_mem`), each naming the terminal whose tree holds it; `machine` asks that machine, over its held link.

```json
{"jsonrpc":"2.0","id":1,"method":"machine.stats","params":{"since":1762416000,"top":1}}
{"jsonrpc":"2.0","id":1,"result":{"at":1762416090,"cpus":8,"disk_free_mb":120000,"disk_total_mb":460000,"history":[{"at":1762416060,"cpus":8,"disk_free_mb":120000,"disk_total_mb":460000,"load_per_cpu":0.25,"mem_free_mb":9500,"mem_pressure_pct":42,"mem_total_mb":16384,"swap_out_pages":12000,"swap_used_mb":512,"terminals":{"exited":1,"working":2},"uptime_s":86400}],"load_per_cpu":0.25,"mem_free_mb":9500,"mem_pressure_pct":42,"mem_total_mb":16384,"swap_out_pages":12000,"swap_used_mb":512,"terminals":{"exited":1,"working":2},"top_cpu":[{"command":"cargo test --workspace","cpu_pct":85,"depth":1,"elapsed_s":600,"own":false,"pid":4242,"ppid":4200,"rss_mb":1200,"terminal":"box/lane-1"}],"top_mem":[{"command":"cargo test --workspace","cpu_pct":85,"depth":1,"elapsed_s":600,"own":false,"pid":4242,"ppid":4200,"rss_mb":1200,"terminal":"box/lane-1"}],"uptime_s":86400}}
```

### `read`

The visible screen, or with `history` the scrollback and screen, as plain text; `lines` keeps the last N lines. Every read returns a `cursor`; with `since: <cursor>` only what is new: the lines that scrolled off since (`lost: N` when scrollback dropped N of them first), then the screen only when it changed (`changed`), so nothing new is an empty `text`. ACP: the transcript as text, or with `events` its events (each with its `seq`), paged by `before` and `n`, or with `since` only those after its `seq`.

```json
{"jsonrpc":"2.0","id":1,"method":"read","params":{"history":false,"lines":2,"name":"build","since":{"screen":2166136261,"seq":40}}}
{"jsonrpc":"2.0","id":1,"result":{"changed":true,"cursor":{"screen":3735928559,"seq":41},"text":"ok 12 tests\n$"}}
```

### `resize`

Resize the terminal; the program gets SIGWINCH.

```json
{"jsonrpc":"2.0","id":1,"method":"resize","params":{"cols":120,"name":"build","rows":50}}
{"jsonrpc":"2.0","id":1,"result":{}}
```

### `rm`

Remove an exited terminal, its ACP session closed; returns it as it was.

```json
{"jsonrpc":"2.0","id":1,"method":"rm","params":{"name":"build"}}
{"jsonrpc":"2.0","id":1,"result":{"cmd":["make","test"],"cwd":"/home/user/project","exit":0,"keep":true,"labels":{"issue":"7","role":"impl"},"name":"build","parent":{"name":"lead","notify":true},"pid":null,"state":"exited"}}
```

### `rpc.discover`

The whole API as one OpenRPC document, as `shed schema` prints it (`docs/shed.openrpc.json`): every method's params and result, the streamed events and screen updates, and the error codes, as JSON Schema. Needs no `hello`.

```json
{"jsonrpc":"2.0","id":1,"method":"rpc.discover","params":{}}
{"jsonrpc":"2.0","id":1,"result":{"info":{"title":"shed","version":"0.0.0","x-protocol":5},"openrpc":"1.3.2"}}
```

### `run`

Start a terminal. `name` defaults to the first free `tN`; `keep` brings it back as a shell in its cwd after a server restart; `rows` and `cols` default to `$SHED_SIZE` (`RxC`) of the server, else 50x200. ACP: `mcp_servers` (stdio, `http`, `sse`, in ACP's shapes) go to the agent; their env and header values are never saved or shown. `meta` (an object) goes unchanged as the session's `_meta`. `no_hibernate` keeps the agent running when idle. `mem_max_mb` (default the server's `$SHED_MEM_MAX`, e.g. `4G`) limits the process tree's memory, and a `mem-limit` event (with `notify`, a `[shed] <name> mem-limit …` line to the parent) says what was ended: on Linux with a user systemd that has the memory controller the program runs in its own scope with `MemoryMax` and the kernel ends a process (the agent's children are ranked first); elsewhere, once the tree is over for two samples in a row, the sampler ends the largest process the agent started (SIGTERM, SIGKILL 2 s later), and an agent over it by itself is only reported. The sampler is a guard against sustained use, not a hard limit: a fast allocation can pass it between samples. `warm` takes the ready spare session of the same recipe (agent, command, cwd, policy, MCP servers, meta, size), renamed to `name` with these labels, and starts the next spare; spares are listed as `.warm-N`, at most `$SHED_WARM_MAX` (default 2) of them, each killed after `$SHED_WARM_IDLE` seconds (default 600) unused. `parent` links it to the terminal that started it: when the parent's program ends the child is killed too, unless `outlive`; with `notify` a `[shed] <name> blocked|error|exited …` line is typed into the parent (a prompt for an ACP parent) when the child blocks, errors or ends, once per piece of news, held while the parent reports `working`. `account` (ACP, on the main host) runs the agent on that account of its provider (`account.ls`): its folder variable set, its token too while its own login is signed out, none of the record's `scrub` and `cache.api_vars` inherited, and a caller `env` setting any of them refused; `meta` and the command are unchanged, and the token is kept in memory only.

```json
{"jsonrpc":"2.0","id":1,"method":"run","params":{"cmd":["make","test"],"cwd":"/home/user/project","keep":true,"labels":{"issue":"7","role":"impl"},"mem_max_mb":4096,"name":"build","parent":{"name":"lead","notify":true}}}
{"jsonrpc":"2.0","id":1,"result":{"cmd":["make","test"],"cwd":"/home/user/project","exit":null,"keep":true,"labels":{"issue":"7","role":"impl"},"name":"build","parent":{"name":"lead","notify":true},"pid":4242,"resources":{"cpu_pct":3,"rss_mb":52},"state":"running"}}
```

### `screen.watch`

Stream a running terminal's screen. After the response the server writes screen updates (see Screen updates), at most 20 a second: the full screen first (none when `since` is the screen's current `seq`), then the rows that changed. It closes the connection when the program's PTY closes. The connection still takes requests; their responses are not written.

```json
{"jsonrpc":"2.0","id":1,"method":"screen.watch","params":{"since":41,"terminal":"build"}}
{"jsonrpc":"2.0","id":1,"result":{}}
```

### `send`

Paste text into the terminal, then press enter: bracketed when the program has turned bracketed paste on, unless `no_paste`; no enter with `no_enter`. While a person has an unsent line (typed through `terminal.input`), the send waits until they press enter or clear it (`ctrl+u`, `ctrl+c`), or `DRAFT_HELD` once `draft_wait` seconds (default 60, 0 off) pass after their last key, writing nothing. Sends go in one at a time, each whole. ACP: the text goes to the bridge as it is.

```json
{"jsonrpc":"2.0","id":1,"method":"send","params":{"name":"build","text":"make lint"}}
{"jsonrpc":"2.0","id":1,"result":{}}
```

### `state`

Report the terminal's state (`working`, `blocked`, `done` or `error`) with an optional note; a program can also print `ESC ] 7501 ; state=<s>[;note=<n>] BEL`, which never reaches the screen. The terminal also speaks the Program Status Protocol (OSC 7501, revision 0.4): `state=<s>[:<key>=<value>]…` with `idle` and `clear`, `id`, `kind`, `progress`, `app` and base64 `title` and `msg`, and answers the probe `ESC ] 7501 ; ?`. A report with an `id` is a record of its own (`records`); on exit `working` and `blocked` ones are dropped.

```json
{"jsonrpc":"2.0","id":1,"method":"state","params":{"name":"build","note":"tests pass","state":"done"}}
{"jsonrpc":"2.0","id":1,"result":{"cmd":["make","test"],"cwd":"/home/user/project","exit":null,"keep":true,"labels":{"issue":"7","role":"impl"},"name":"build","parent":{"name":"lead","notify":true},"pid":4242,"reported":{"note":"tests pass","state":"done"},"resources":{"cpu_pct":3,"rss_mb":52},"state":"running"}}
```

### `terminal.input`

Type text into the terminal as a person: written as it is, and a line left unsent holds `send`s until enter, `ctrl+u` or `ctrl+c`.

```json
{"jsonrpc":"2.0","id":1,"method":"terminal.input","params":{"terminal":"build","text":"make lint"}}
{"jsonrpc":"2.0","id":1,"result":{}}
```

### `terminal.keys`

Press keys by name as a person, as `keys`; `ctrl-x` is `ctrl+x`. Like `terminal.input`, a line left unsent holds `send`s.

```json
{"jsonrpc":"2.0","id":1,"method":"terminal.keys","params":{"keys":["esc","ctrl-c","up","enter"],"terminal":"build"}}
{"jsonrpc":"2.0","id":1,"result":{}}
```

### `terminal.procs`

The terminal's live process tree, parents first, each with its `depth` below the terminal's own process, its CPU (percent of one core over its life), memory, age and command line; `own` marks the agent's own processes, not work it started. A terminal that is not running has none.

```json
{"jsonrpc":"2.0","id":1,"method":"terminal.procs","params":{"name":"lane-1"}}
{"jsonrpc":"2.0","id":1,"result":[{"command":"node agent.js","cpu_pct":3,"depth":0,"elapsed_s":600,"own":true,"pid":4200,"ppid":1,"rss_mb":180},{"command":"cargo test --workspace","cpu_pct":85,"depth":1,"elapsed_s":600,"own":false,"pid":4242,"ppid":4200,"rss_mb":1200}]}
```

### `terminal.readonly`

Mark the terminal read-only for this connection: from then on, until it closes, the connection's `send`, `keys`, `resize`, `attach`, `terminal.input`, `terminal.keys` and `terminal.resize` to the terminal are refused with `READONLY`. The connection may resize only the terminals it has not marked.

```json
{"jsonrpc":"2.0","id":1,"method":"terminal.readonly","params":{"terminal":"agent"}}
{"jsonrpc":"2.0","id":1,"result":{}}
```

### `terminal.resize`

Resize the terminal, as `resize`; without `terminal`, the one this connection watches.

```json
{"jsonrpc":"2.0","id":1,"method":"terminal.resize","params":{"cols":60,"rows":30}}
{"jsonrpc":"2.0","id":1,"result":{}}
```

### `wait`

Answer once the reported state is one of `until` (`idle` too, from the `idle_after` fallback; `blocked` only of `kind`, if given) or a visible screen line matches the pattern `output` (`matched`), the program exits (`exited`) or `timeout` seconds pass (`timeout`). The server blocks; nothing polls. With `children`, `name` is a parent: `all` matches once no child is working (`terminal` is the parent), `any` on the next child that reports `done`, `blocked` or `error`, or exits (`terminal` is that child; `exited` when no child runs).

```json
{"jsonrpc":"2.0","id":1,"method":"wait","params":{"kind":"question","name":"build","output":"^ok \\d+ tests$","timeout":600.0,"until":["done","blocked"]}}
{"jsonrpc":"2.0","id":1,"result":{"outcome":"matched","terminal":{"cmd":["make","test"],"cwd":"/home/user/project","exit":null,"keep":true,"labels":{"issue":"7","role":"impl"},"name":"build","parent":{"name":"lead","notify":true},"pid":4242,"records":[{"id":"build/tests","state":"working"}],"reported":{"app":"make","kind":"question","msg":"Overwrite config.toml?","progress":40,"state":"blocked","title":"Docs"},"resources":{"cpu_pct":3,"rss_mb":52},"state":"running"}}}
```

## Events

An event names its terminal, carries its labels and its `parent` (absent without one) and counts up in
`seq`; `event` is `started`, `state` (the reported state changed), `exited` (by itself, with its `code`),
`killed` (by `kill`), `resources` (memory or CPU moved by more than 10 % since the last one) or `mem-limit`
(the tree passed `mem_max_mb`; `ended_by` is `kernel`, `shed` or `none`).

An ACP terminal (`shed run --acp`) adds its agent's events, emitted by its bridge: `turn` (a prompt), `text`
(a reply chunk), `tool_call` (started or changed), `plan`, `mode`, `approval_request` (answer with
`shed approve`), `approved`, `reply` (the turn's whole reply and its `stop_reason`), `session` (the session
is ready, or its model or slash commands changed) and `usage` (the agent's `usage_update`: tokens `used` of
the context window's `size`, and its `cost`). A resumed session's history (`run` with `resume`) comes first, from
shed's own transcript, its events marked `"replay": true` so a client does not count them as new; what the
agent replays is dropped. An idle agent hibernates (state `hibernated`, its processes stopped); the next
send wakes it on the same session.

```json
{"seq":1,"name":"build","labels":{"issue":"7","role":"impl"},"parent":"lead","event":"started"}
{"seq":2,"name":"build","labels":{"issue":"7","role":"impl"},"parent":"lead","event":"state","state":"done","note":"tests pass"}
{"seq":3,"name":"build","labels":{"issue":"7","role":"impl"},"parent":"lead","event":"resources","rss_mb":52,"cpu_pct":3}
{"seq":4,"name":"build","labels":{"issue":"7","role":"impl"},"parent":"lead","event":"mem-limit","limit_mb":4096,"process":"cc1plus","rss_mb":4301,"ended_by":"shed"}
{"seq":5,"name":"build","labels":{"issue":"7","role":"impl"},"parent":"lead","event":"exited","code":0}
{"seq":6,"name":"build","labels":{"issue":"7","role":"impl"},"parent":"lead","event":"killed"}
{"seq":7,"name":"build","labels":{"issue":"7","role":"impl"},"parent":"lead","event":"turn","prompt":"Fix the failing test"}
{"seq":8,"name":"build","labels":{"issue":"7","role":"impl"},"parent":"lead","event":"text","text":"Looking at the test"}
{"seq":9,"name":"build","labels":{"issue":"7","role":"impl"},"parent":"lead","event":"tool_call","id":"call_1","title":"Edit src/lib.rs","status":"pending","kind":"edit","locations":["/work/src/lib.rs"],"raw_input":{"file_path":"/work/src/lib.rs"}}
{"seq":10,"name":"build","labels":{"issue":"7","role":"impl"},"parent":"lead","event":"plan","entries":[{"content":"Fix the test","priority":"high","status":"in_progress"}]}
{"seq":11,"name":"build","labels":{"issue":"7","role":"impl"},"parent":"lead","event":"mode","mode":"default"}
{"seq":12,"name":"build","labels":{"issue":"7","role":"impl"},"parent":"lead","event":"approval_request","id":"7","tool":"Edit src/lib.rs","options":[{"id":"allow","name":"Allow","kind":"allow_once"},{"id":"reject","name":"Reject","kind":"reject_once"}],"call":{"toolCallId":"call_1","kind":"edit","locations":[{"path":"/work/src/lib.rs"}]}}
{"seq":13,"name":"build","labels":{"issue":"7","role":"impl"},"parent":"lead","event":"approved","option":"allow"}
{"seq":14,"name":"build","labels":{"issue":"7","role":"impl"},"parent":"lead","event":"reply","text":"Fixed; all 12 tests pass.","stop_reason":"end_turn"}
{"seq":15,"name":"build","labels":{"issue":"7","role":"impl"},"parent":"lead","event":"session","id":"sess_1","driver":"acp","model":"opus","models":[["opus","Opus"],["haiku","Haiku"]],"mcp_servers":[{"headers":[{"name":"Authorization","value":"***"}],"name":"web","type":"http","url":"https://mcp.example/"}],"commands":[["review","Review the diff","files"]],"usage":{"used":53000,"size":200000,"cost":{"amount":0.045,"currency":"USD"},"turn":{"input":100,"cache_read":900,"cache_write":1000,"output":50},"total":{"input":200,"cache_read":1800,"cache_write":2000,"output":100}},"turn_ended":1763337600,"cache_lifetime":3600,"cache_likely":"warm","ready_ms":2400,"first_token_ms":1800}
{"seq":16,"name":"build","labels":{"issue":"7","role":"impl"},"parent":"lead","event":"usage","used":53000,"size":200000,"cost":{"amount":0.045,"currency":"USD"},"turn":{"input":100,"cache_read":900,"cache_write":1000,"output":50},"total":{"input":200,"cache_read":1800,"cache_write":2000,"output":100}}
```

## Session records

A session shed ran, `sessions/<id>.json` beside the state file: `agents sessions` lists these, `run
--resume <id>` continues one. Its transcript is `sessions/<id>.jsonl`; in a file name each byte of `id`
outside `[A-Za-z0-9._-]` is `%XX`. Kept `$SHED_SESSIONS_KEEP` days (default 30) after its last turn; `rm`
of its terminal deletes both at once.

```json
{"id":"sess_1","agent":"claude-acp","cwd":"/work","title":"Fix the failing test","updated":1763337600,"turns":3,"transcript":"/run/shed/sessions/sess_1.jsonl"}
```

## Screen updates

`screen.watch` writes one update per line. `seq` is the screen's change count; `rows`, `cols`, `cursor`
(`[row, col]`, from 0) and `scrollback` (lines) are always there; `lines` holds the rows that changed, each
as runs of cells with one pen: `fg` and `bg` (a palette index 0–255, or `[r, g, b]`; `null` is the default),
`bold`, `italic`, `underline` and `inverse`. Trailing blank default cells are dropped, so a blank row has no
runs. `full` marks an update that carries every row: the first, and the one after a resize.

```json
{"seq":41,"full":true,"rows":2,"cols":40,"cursor":[1,2],"scrollback":120,"lines":[{"row":0,"runs":[{"text":"ok","fg":2,"bg":null,"bold":true,"italic":false,"underline":false,"inverse":false},{"text":" 12 tests","fg":null,"bg":null,"bold":false,"italic":false,"underline":false,"inverse":false}]},{"row":1,"runs":[{"text":"$","fg":null,"bg":null,"bold":false,"italic":false,"underline":false,"inverse":false}]}]}
{"seq":44,"full":false,"rows":2,"cols":40,"cursor":[1,6],"scrollback":120,"lines":[{"row":1,"runs":[{"text":"$ ","fg":null,"bg":null,"bold":false,"italic":false,"underline":false,"inverse":false},{"text":"make","fg":null,"bg":[40,40,40],"bold":false,"italic":false,"underline":false,"inverse":false}]}]}
```

## Errors

An error's `message` starts with its name: `NAME: detail`.

| Code | Name |
|---|---|
| -32000 | `SHED_VERSION` |
| -32001 | `NO_HELLO` |
| -32002 | `NO_SUCH_TERMINAL` |
| -32003 | `NAME_TAKEN` |
| -32004 | `STILL_RUNNING` |
| -32005 | `SPAWN_FAILED` |
| -32006 | `READONLY` |
| -32007 | `NO_LINK` |
| -32008 | `IO` |
| -32009 | `DRAFT_HELD` |
| -32010 | `HANDOVER_REFUSED` |
| -32011 | `LIMIT` |
| -32012 | `NO_ACCOUNT` |
| -32013 | `ACCOUNT_SETUP` |
| -32014 | `ACCOUNT_SIGNED_OUT` |
| -32015 | `ACCOUNT_LIMITED` |
| -32016 | `NOT_DELIVERED` |
| -32600 | `INVALID_REQUEST` |
| -32601 | `NO_SUCH_METHOD` |
| -32602 | `INVALID_PARAMS` |
| -32603 | `INTERNAL` |
| -32700 | `PARSE` |

```json
{"jsonrpc":"2.0","id":1,"error":{"code":-32000,"message":"SHED_VERSION: client 02fcf97 protocol 6-7 / server 56de739 protocol 4-5; update the server and restart shed serve"}}
```
