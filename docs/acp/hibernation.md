# Hibernation

An idle agent hibernates: once its session has had a turn (or was resumed) and has been `done` (or, resumed,
has had no turn yet) for `$SHED_ACP_HIBERNATE_AFTER` seconds in the environment of `shed serve` (default 600, or
the record's prompt cache lifetime on this login when that is shorter; `0` never), the bridge stops the agent's process tree and reports the state `hibernated` (a `state` event). The
terminal, its session id, model, models and slash commands stay; only the bridge (about 8 MB) is left. The next
send (a prompt, `model`, …) starts the agent again with the same command, folder and environment, loads the
session (`session/load`, what the agent replays dropped: the history is shed's transcript already) and then
delivers it; sends that arrive meanwhile follow in order. The MCP servers stay in the bridge's memory
only. An agent without `loadSession`, or a `run --acp … --no-hibernate` (the `run` method's `no_hibernate`),
never hibernates. Measured on 2026-11-07: ten idle fake agents took 197,248 KiB awake and 77,568 KiB
hibernated (wake 76 ms); one claude-agent-acp 0.100.0 took 502,700 KiB in 7 processes awake and 8,064 KiB in 1
hibernated, and woke to its first update in 1,946 ms (`crates/shed/tests/acp_bench.rs`, `hibernate` and
`hibernate_live`).
