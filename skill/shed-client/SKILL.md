---
name: shed-client
description: Talk to a shed server from Rust with the shed-client crate (connect, typed calls, streams, errors, the version contract), read agent records with shed-catalog, and test against shed_client::fake. Use when an app starts or drives shed terminals or ACP lanes, follows shed events, or needs a fake shed in its tests.
---

# shed-client

One blocking client for shed's socket, with no async runtime. It reads no clock and no environment and
starts no process: the caller passes the socket, does its own retry sleeps and its own logging.

```toml
[dependencies]
shed-client = { git = "https://github.com/jirathip-dev/shed", rev = "<rev>" }
shed-catalog = { git = "https://github.com/jirathip-dev/shed", rev = "<rev>" }   # the agent records

[dev-dependencies]
shed-client = { git = "https://github.com/jirathip-dev/shed", rev = "<rev>", features = ["fake"] }
```

Pin both crates at one rev; `shed_client::proto` re-exports the protocol types, so add no `shed-proto` of
your own.

## Connect

- `Shed::at(socket)`: a server with a pool of at most four idle connections and a 10 s call timeout
  (`.timeout(None)` waits for ever). Nothing is dialled until the first call; a pooled connection whose
  peer went away (a restart, a handover) is dropped and dialled again, and a request is never sent twice.
- `shed.hello()`: the server's version, build and protocol.
- `shed.connect()` / `Conn::connect(path)` / `Conn::over(stream)`: a connection of your own, past the
  handshake; `over` takes any `UnixStream`, such as one end of a socketpair to `ssh host shed connect`.
- `shed_client::once(method, &params, exchange)`: one call over a transport that carries one exchange; it
  sends the hello and the call as lines and reads every line back.
- `impl Calls for YourTransport { fn call_raw(..) }` gives every typed call below on that transport.

## The calls, by domain (`use shed_client::Calls as _`)

- Terminals: `run(&RunParams)`, `ls`, `get`, `read`, `screen`, `send`, `keys`, `resize`, `state`, `kill`,
  `rm`, `close` (kill then rm; already gone is fine), `wait`, `wait_state(name, &[states], secs)`,
  `wait_output(name, pattern, secs)`. A `wait`'s deadline is its own timeout plus the call timeout.
- ACP: `ready(name, secs)` (the session, once the bridge says so), `prompt`, `approve(name, option,
  request)`, `cancel`, `model`. `run` takes `acp`, `model`, `effort` and `env`: shed turns the model and
  effort into the agent record's words and keeps `env`'s values in memory only.
- Agents: `agents()`, `matched(id, text)` (an agent's limit and sign-in rules on a text).
- Machines: `links()`, `exec`, `stats`; files: `file_read`, `file_write`; `emit`, `handover_check`,
  `handover`.
- Every params type derives `Default`: name the fields you set and end with `..RunParams::default()`.

Streams run on a connection of their own and never return to the pool: `shed.events(&EventsParams)`,
`shed.watch(name, since)` and `shed.attach(name)`. A `Stream<T>` is a blocking iterator of
`Streamed::Item(T) | Reset(seq) | Lagged(seq) | Quiet | Skipped(line)`; `.tick(every)` turns a silent stretch
into `Quiet` so the loop can look up, and `stream.since()` is where to resume. Reconnecting is your loop.

## Errors

One `shed_client::Error`:

- `Io { socket, source }`: nothing listens, the connection broke, or no answer within the timeout.
- `Version { .. }`: not shed, a server outside this client's protocols, or a call this server is too old
  for. Its text is one line naming the side to update; show it as is.
- `Rpc(RpcError)`: the server's own error; `e.code() == Some(ErrorCode::NoSuchTerminal)` and the like.
- `Bad(text)`: an answer that could not be read.

## The version contract

`shed_client::MIN_PROTOCOL` is the oldest server protocol this client works against.

| the server | you get |
|---|---|
| not shed, or sends no protocol | `Version` at connect |
| under `MIN_PROTOCOL` | `Version` at connect |
| at or above it, lacking a method or field the call uses | `Version` on that call, naming it (`run.env`); every other call works |
| at or above it and knows the call | the answer |
| newer, still serving this client's protocol | the answer, in this client's shapes |
| newer, no longer serving it | `Version` at connect: update the app |

From protocol 3 a server refuses a request field it does not know, so an old server fails loudly instead
of dropping it. shed updates itself from its own releases; an app moves its pin when it needs a new call.

## The catalog's layers (`shed-catalog`)

```
built-in records      compiled in: Catalog::built_in(), no server, no file
      ▲ key by key
the host's files      <config>/providers/<agent>.toml, every app on that host: Catalog::load(dir), or
                      Catalog::served(&shed.agents()?) for what a server sees
      ▲ key by key
an app's rows         catalog.over(agent, toml): that app only
```

Each upper layer is a table of keys over the record of that name, checked as a whole record afterwards; an
unknown key is an error naming the file or the row. Look a record up with `by_id` (the ACP id) or
`by_agent` (the program's name); `iter()` gives each with its source.

## Testing: `shed_client::fake` (feature `fake`)

`fake::Server::start()?` serves a real socket in a temp folder of its own, removed when it drops. It
answers `hello` as shed does and any method it was not given `MethodNotFound` (so the client reports
`Version`, naming the method). `answer(method, json)` sets a result, `fail(method, RpcError)` an error;
for `events`, `screen.watch` and `attach` a JSON array is the stream's items, one line each, after which
the stream ends. `seen()` is every call but `hello`, in order, with its params.

## Examples

Compiled with the crates on every check, so they cannot go stale:

- `examples/lane.rs`: start an ACP lane, prompt it, wait for its turn, read the screen, close it.
- `examples/events.rs`: follow labelled events and resume after the stream closes.
- `examples/catalog.rs`: the built-in records with an app's row over one.
- `examples/fake.rs`: a test against the fake server.

`shed skill --to <dir>` writes this folder to `<dir>/shed-client/` from the shed binary.
