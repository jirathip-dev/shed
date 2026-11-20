# ADR: one shed client and one provider catalog, both shed's (#191)

## Summary

Decision: shed gets two crates beside shed-proto. shed-client is the one socket client, blocking and with no async runtime: connect and version check, typed calls for terminals, ACP sessions, streams, agents and machines, one error type. shed-catalog holds the agent records (program, commands, models, model and effort words, limit and sign-in lines), built in and overridable per host and per app. shed's own CLI and server use both first; cooee and drover then delete their copies. One minimum protocol lives in the client, and from protocol 3 a server refuses a request field it does not know, so an old server fails loudly instead of dropping it.
Slices: (1) the client crate, with shed's CLI and server moved onto it; (2) protocol 3 and the one minimum; (3) the catalog crate and the records' new keys; (4) run takes a model, an effort and an environment; (5) a test server and the skill. Then four adoption issues: cooee client, cooee catalog, drover client, drover catalog. About +690 lines in shed; about 1,050 fewer in cooee and drover, and 424 fewer test lines.
Open questions: (1) may run take a model, an effort and an environment, so shed builds the agent's command (recommended), or do the apps keep building it from catalog data? (2) Raise shed's line cap from 13,250 to 13,950 before slice 1 (recommended)? (3) Set the shed fleet's done check to the one command CI runs, just check (recommended)?

**Status:** proposed. Usage, shapes, boundaries, the statechart, the inventory, the agent-speed mechanisms
and the slices: [README.md](README.md).

## The question

Three programs speak shed's socket protocol and each wrote its own client: shed's CLI and server
(`shed_server::Client`, 95 lines), cooee (`shed/`, 887 lines) and drover (`shed.rs`, 349 lines, plus a
424-line fake server in its tests). Three repos also keep facts about the same agent programs: shed's
provider records, cooee's `provider.rs` and `providers/` (903 lines), drover's `providers/` (1,165 lines).

What the copies cost today, all found in the code on 2026-11-20:

- The handshake and its refusal text exist three times (`fn side(hello)` twice, printing the same text), with two
  hand-kept minimum protocols (`SHED_MIN_PROTOCOL`, `shed::MIN_PROTOCOL`, both 1) that no test holds true.
- The framing of an ACP prompt (`RS`, JSON, newline, after a `working` report) exists in shed's CLI and in
  cooee; drover starts the `shed` CLI for every prompt and cancel to avoid a third copy.
- Both apps start ACP lanes through the CLI, not the socket, so MCP servers and secrets pass through temp
  files and a command line. drover finds out whether a host's shed takes `--meta` by reading
  `shed help run`, and reads `shed agents --json` in two shapes.
- The facts disagree: the login command of one agent (`claude auth login` in shed and cooee, `claude /login`
  in drover), the id of another (`gemini` in cooee, `gemini-acp` in drover), its install command, the
  effort lists. Each app also has its own way to turn a model and an effort into words, and neither can
  know which driver the host's record picked, so neither can place them right for the stream driver.
- Every `RunParams` field shed adds breaks both apps' builds: each writes out every field.

So: what is the one client, what is the one catalog, where does each sit, and what do the apps keep?

## Options

**The client**

| | option | verdict |
|---|---|---|
| A | Leave the three clients | every protocol change is three lanes in three repos |
| B | The apps call the `shed` CLI with `--json` | a process per call, no streams, secrets on command lines; both apps already left it for the socket where they could |
| C | **A blocking, std-only `shed-client` crate that shed's own CLI and server use first** | chosen |
| D | An async client on tokio | all four callers are blocking today (the CLI by decision 0004, the server's machine links, both apps); it would put a runtime into two apps and slow the crate's gate |
| E | Generate the client from the OpenRPC document | the document has the shapes; what was copied is the rest: handshake, pool, framing, streams, errors |

**Where the catalog sits**

| | option | verdict |
|---|---|---|
| F | In shed-proto | the record's types are there and stay there (they are on the wire); the data would give every user of the wire types a TOML parser and a build script |
| G | In shed-client | a client is sockets; the server would depend on it for data, and the records' tests would sit behind them |
| H | Served only (`agents`), nothing compiled in | an app needs facts with no server running: a config check, a hook, a doctor row |
| I | **Its own crate, `shed-catalog`: the built-in records and the layering, no sockets** | chosen |

**The version contract**

| | option | verdict |
|---|---|---|
| J | Each app keeps its own minimum (today) | two numbers nothing tests, and feature probing beside them |
| K | The minimum is the protocol of the app's pin | no code, but every pin bump stops lanes on every host whose shed has not updated yet |
| L | **One floor in shed-client, and a server that refuses what it does not know** | chosen |

## Decision

1. **`shed-client`**, a crate next to `shed-proto`, depending on it, serde and thiserror only. Blocking.
   `Conn` is one connection past the handshake. `Shed` is a server: its socket, a pool of at most four idle
   connections and a call timeout. The typed calls are provided methods of one trait, `Calls`, over one
   required method, so they are written once for a pooled socket and for a transport that carries one
   exchange per connection (`once`: drover's `ssh host shed connect`). Streams (`events`, `screen.watch`,
   `attach`) are blocking iterators, each on a connection of its own. The crate reads no clock, no
   environment and starts no process: retry loops, autostart and the socket's default path take what they
   need from the caller.
2. **One error type.** `Io` (nothing listens, or the connection broke), `Version`, `Rpc` (the server's
   error, with `code()` as an `ErrorCode`), `Bad` (an answer that cannot be read). The `SHED_VERSION` text
   and its fix are written once, in the client.
3. **shed is the first user.** `shed_server::Client` is deleted; the CLI, the machine links and the bridge
   use `shed-client`. The prompt framing (`Input`, `frame`) moves to shed-proto so the client can write it.
4. **The version contract.**
   - `shed_client::MIN_PROTOCOL` is the one floor, 3. A client sends it as `hello`'s `oldest`. No app
     defines a protocol number.
   - Protocol 3: a server refuses a request field it does not know (`INVALID_PARAMS`), checked once in
     dispatch against the method's documented fields; a connection on protocol 1 or 2 is served as before.
   - So above the floor a newer client on an older server fails on the one call that needs more, with
     `Version` naming the method or field, and never silently. `shed help run` probing is deleted.
   - `PROTOCOL` moves only on a breaking change (a method, field or meaning removed or changed); additions
     do not move it. The server keeps answering `OLDEST_PROTOCOL..=PROTOCOL` in each client's shapes, so
     servers update first and clients follow. `just protocol` fails a PR that drops or retypes anything in
     `docs/shed.openrpc.json` without raising `PROTOCOL`.
   - A client skips a stream line it cannot read and says so (`Streamed::Skipped`), so a new event kind is
     an addition.
5. **`shed-catalog`**, a crate depending on `shed-proto` and `toml`. It holds the built-in records
   (`crates/shed-catalog/providers/<agent>.toml`, moved from shed-server, collected by its build script)
   and `Catalog`, three layers with one merge rule, key by key: the built-in records, under the host's
   files in `<config>/providers/`, under an app's own rows. `Catalog::built_in()` needs no server;
   `Catalog::served(&shed.agents()?)` is what the host's server holds, and wins when a lane starts there.
6. **What a record holds** is what is true of the agent program and either shed acts on it or two apps
   need it. New keys: `install`, `token_var`, `models` (moved up from the stream table), `model_args`,
   `model_env`, and `effort` in the stream table. New record: gemini, with one id for both apps.
   `Provider::pick` turns a model and an effort into variables and words for the driver that runs.
   What an app does with an agent stays in the app, keyed by the record's id: cooee's persona setup and its
   status and usage readers, drover's hook dialects, profile sync, sandbox needs and token proxy. A fact
   moves into the record when a second repo needs it.
7. **`run` takes `model`, `effort` and `env`** (open question 1). The server applies `pick` for the driver
   it chose and gives `env` to the program; `env` stays in memory, like `mcp_servers`. `cmd` stays for a
   caller that needs a wrapper.
8. **Every params type derives `Default`**, so a caller names the fields it sets and a new field breaks no
   build.
9. **A test server** (`shed_client::fake`, behind the `fake` feature) and **a skill** (`skill/shed-client/`,
   written out by `shed skill --to <dir>`), whose examples are the crate's compiled examples.

## Consequences

- **A protocol change is one lane in one repo.** The typed call, the server and the CLI change together and
  shed's own tests drive the real server through the client. An app sees it when it moves its pin.
- **Every host's shed is on protocol 3 before an app adopts.** shed updates itself, so the order is: shed's
  slices, a release, then the adoption issues. An app on the client refuses an older server with one line
  and the fix.
- **Both apps start ACP lanes over the socket.** MCP servers and secrets no longer touch a temp file or a
  command line; cooee's secrets file and its `sh -c` reader go with slice 4.
- **The records settle the disagreements.** The adoption lanes pick one login command, one id and one
  install command per agent; an app that wants another writes one row, in its own config.
- **A compiled-in record can be older than the host's.** An app checks its config against `built_in()` and
  launches by `served()`. With slice 4 it does not read the launch keys at all.
- **Without slice 4** the apps call `Provider::pick` and keep building `env K=V … adapter words…`
  themselves; the catalog still gives one set of facts, but the stream driver keeps its adapter swap and
  cooee's effort words keep a lane on ACP.
- **`shed-proto/src/lib.rs` is 60 lines under the 2,000-line file limit and `registry.rs` 24.** Nothing
  here is added to either: the record types move to `shed-proto/src/agents.rs` with their new keys, the
  input framing to `input.rs`, and slice 4's launch code goes to `drivers/launch.rs`.
- **The server's `lib.rs` loses 95 lines**, which replaces the `client.rs` row of the hotspots ADR's
  slice 4.
- **The cap.** shed is at 13,215 of 13,250. The slices add about 690 (README), so one raise to 13,950
  before slice 1 (open question 2). The fake server is 150 of those; it is test support that ships in
  `src/` because two repos link it.
- **Left to the apps, each its own issue later:** drover's lane prologue (its variables can go to `env`,
  its two file copies to `exec`, and then it needs no wrapper and no adapter lookup); a `[terminal]` table
  for drover's terminal launch words, when a second app runs terminal lanes; cooee's status and usage
  readers as record data, when drover reads them too.

## `.drover/arch.toml`

No central list is added, so nothing is frozen. The one list this design touches, the records, stays a
folder collected by a build script (`crates/shed-catalog/build.rs` globs `providers/*.toml`; the server's
keeps globbing `src/providers/*.rs` for the wires): a new agent adds files and edits none. The typed calls
are split by domain, one file each, and a time budget sits in its own crate's `Cargo.toml`, not in a shared
file. Slice 1 adds, in the PR that creates the client:

```toml
[[layer]]
name = "client"
paths = ["crates/shed-client/src/**"]
mods = ["shed_client"]
may_use = ["proto"]

[[ban]]
pattern = "Hello::ours"
paths = ["crates/*/src/**"]
except = ["crates/shed-proto/src/**", "crates/shed-client/src/**", "crates/shed-server/src/lib.rs"]
reason = "shake hands through shed_client; only the server's dispatch answers a hello"
```

and changes two lines: the `server` layer's `may_use = ["core", "proto", "client"]` and the `cli` layer's
`may_use = ["server", "client"]`. Slice 3 adds, in the PR that creates the catalog:

```toml
[[layer]]
name = "catalog"
paths = ["crates/shed-catalog/src/**"]
mods = ["shed_catalog"]
may_use = ["proto"]
```

and the `server` layer becomes `may_use = ["core", "proto", "client", "catalog"]`. The existing bans
(`Command::new`, `std::process`, `thread::sleep`, `env::var`, `SystemTime::now`, `Instant::now`) already
cover `crates/*/src/**`, so both crates are held to them from their first commit with no exception. Each
adoption PR adds one ban to its own repo's `arch.toml`, `pattern = "Hello::ours"` over its sources with no
exception, so a hand-rolled client cannot come back.
