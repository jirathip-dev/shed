# Provider records

What shed knows about an agent is one TOML file, never code: `crates/shed-catalog/providers/<name>.toml`
is built in (the `shed-catalog` crate, which an app reads with no server: `Catalog::built_in`, `load`, `served` and `over`, an app's own row on top), and `<name>.toml` in `providers/` beside the config file (`~/.config/shed/providers/`, or
`~/Library/Application Support/shed/providers/` on macOS) overrides its keys one by one or adds an agent,
read each time a lane starts, so a changed banner needs no rebuild. Keys: `id` (the ACP agent id), `agent`
(the agent program's own name), `auth_failures`, `limit`, `resets`, `login`, `status` and `logout` (commands), `install` (the command that installs the program), `token_var` (the variable that carries a login token), `config_dir_var`, `settings_var`, `caller_var` and `caller` (with `caller = true` a lane's adapter or stream driver starts with `caller_var` set to `shed`, or to the record's `env` value for it, a run's own `--env` winning over both; off, shed sets nothing and the adapter's own value stands),
`models` (`[id, name]` pairs), `model_args` (`{model}` stands for the pick) or `model_env` (the variable that sets it), `effort_args` (`{effort}` stands for the level), `efforts`, the `[stream]` table's `model` and `effort` for the stream driver (`Provider::pick` places a model and an effort for the driver that runs), `[quota]` (`path`, the keys from a prompt response's `_meta` down to the turn's counters, and the `input`, `cache_read`, `cache_write` and `output` keys in it; see [Run one](run.md)) and `[cache]` (`plan` and `api`, the prompt cache's lifetime in seconds on each login, and `api_vars`, the variables that mean an API key; see [Prompt cache](prompt-cache.md)). A user file that is not TOML or not a record
is skipped, the built-in record staying in force, and `shed doctor` names it (`FAIL provider …`).

`shed agents --json` and the `agents` method list every agent with a record or an installed adapter: `id`,
`source` (`built-in` or the user file), `provider` (the record), `installed` and `medians`. `agents.match {id, text}`
applies an agent's limit and auth rules to text of a caller's own (a pane's screen) and returns the `limit`
line, its `resets` time and the `auth` line, as a lane would read them.

Each ACP terminal's `session` in `ls --json` and `get` has `ready_ms`, the time from `run` to the session being
ready (the agent started, `initialize` and `session/new` or `session/load` answered), and `first_token_ms`, the
time from its first prompt to the first chunk of the reply (a tool call before it does not count). Both are
taken once per terminal, so a wake from hibernation does not change them. When a session has both, shed adds
them to `timings.jsonl` beside the state file, and `medians` in `agents` is each agent's `sessions`,
`ready_ms` and `first_token_ms`: the medians over its last `$SHED_MEDIAN_OF` (default 20) such sessions.
