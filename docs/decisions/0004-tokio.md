# 0004: The server on one tokio runtime

**Status:** accepted (#42, after #40)

## Context

The server ran a thread per connection, per stream (`attach`, `events`, `screen.watch`), per running terminal
watcher and per `screen.watch` diff, plus the sampler and the stats recorder: 3 + 2T + C + S + W threads. Every
`events` client had its own channel, so one event cost one send per client, and every stream wrote and
flushed once per message. #40 put the ACP bridge on the official async SDK, which needs tokio, and glued it to
the sync rest with two threads. 0003 said "no async runtime"; that no longer held.

## Decision

- `shed serve` runs on one multi-thread tokio runtime, `min(cpus, 4)` workers. The accept loop, each
  connection, each stream, each terminal watcher, the sampler and the stats recorder are tasks.
- A connection reads lines and dispatches them under the std `Mutex<Registry>`, never held across an
  `.await`. `wait` awaits in its connection's task; `kill`, `ps`, `df` and `sysctl` run on the blocking pool.
- PTY reads stay on one plain thread per terminal: a terminal reads for hours, which would pin blocking-pool
  threads, and an async PTY would need a non-blocking master that `write` under the registry lock cannot use.
- shed-core takes tokio's `sync`, `time` and `macros` only: a terminal's status is a `tokio::sync::watch`, so
  `wait_until` is async, and output subscribers are tokio channels, both fed from the reader thread.
- Events live in one shared log. `emit` is one push and one wake however many clients listen; each client
  keeps a cursor and writes all that is new as one batch. The log keeps the last 1,000 for `since` and
  everything a live client has not read, so a slow client misses nothing, as before.
- The ACP bridge's PTY reader and writer are tasks now, and the writer sends one batch per wake. Its runtime
  gets one worker for them, so the writer runs beside the session as the old writer thread did: on one
  thread the ACP bench was 27% slower than the base, with the worker it is faster.
- The CLI stays sync: short-lived, unmeasured, and the protocol is unchanged.
- No shutdown handling is added: the server is killed, as before.

## Consequences

- Threads: workers (≤ 4) + one reader per terminal + blocking threads only while a job runs. 20 terminals and
  50 `events` clients: 143 threads before, 24 after.
- `scripts/bench-fanout.sh` also reports 500 idle `events` clients and one state change at the base and the
  head: the time until all have it and the server's thread count.
- A dead client's stream ends when its read half does, not at its next failed write.
- The berth→shed shims (`$BERTH`, the one-time dir move) were dropped to stay under the line cap.
