# Prompt cache

A wake loads the session and the next turn re-sends the conversation; the agent's vendor reads from its prompt
cache the prefix that is unchanged and still within the cache's lifetime, and charges full input price for the
rest. Claude Code's docs give that lifetime as 1 hour for the main conversation on a subscription and 5 minutes on
an API key, usage credits or a cloud provider, and list what drops the cache mid-session: a model switch, an
effort change, an MCP server added or removed while tool search is off, and a CLI upgrade
(<https://code.claude.com/docs/en/prompt-caching>). None of this is measured by shed; `CACHE` in `shed top`
is the number to check it with.

An agent's record says it in `[cache]`: `plan` and `api` seconds, and `api_vars`, the variables that, set and not
empty in the lane's environment, mean an API key (for `claude-acp` `ANTHROPIC_API_KEY`, `ANTHROPIC_AUTH_TOKEN`
and the Bedrock, Vertex and Foundry switches); without one the login counts as a plan. Usage credits on a
subscription are not told apart and count as a plan. The lane's lifetime is `session.cache_lifetime`, and
the bridge hibernates after 600 s or that lifetime, whichever is shorter: an API key lane sleeps once its cache
is likely gone, since a wake after that pays full input price whether or not the process stopped; a plan lane
keeps 600 s (lowering it waits for cache numbers that show a wake inside the hour reads from cache).
`$SHED_ACP_HIBERNATE_AFTER`, when set, overrides it as before.

`session.turn_ended` is when the last turn ended (unix seconds). While the lane is `done`, `idle` or
`hibernated`, `get` shows `session.cache_likely`, `warm` until the idle time reaches the lifetime and `cold`
after, and `shed top` shows it as `CACHE-LIKELY`; it is a likelihood from the clock, not the vendor's word.
An agent without `[cache]` shows neither. `shed model` on a session that has had a turn prints one line on
stderr that the switch drops the prompt cache, and switches.

`shed rm` of an exited ACP terminal closes its session (`session/close`) when the agent advertises
`sessionCapabilities.close`: the agent's process is gone by then, so shed starts the agent once in the
terminal's folder, sends `initialize` and the close, and stops it (at most 30 s; a failure does not stop the
`rm`). Without the capability only the handshake runs. Hibernation never closes a session; it loads it again.
