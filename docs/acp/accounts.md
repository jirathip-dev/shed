# Accounts

shed passes its environment (the server's, `PATH` included) through to the agent and never handles provider credentials. Each agent logs in
its own way (its own login command, config file or API key variable); log in once outside shed first.

**An expired login** ends the turn at once: the state is `error` with the note `auth: <the agent's line>`
(for `claude-acp`, e.g. `auth: Failed to authenticate: OAuth session expired and could not be refreshed`),
and the `reply` event carries that text with `stop_reason` `auth_error`. To recover, check
`claude auth status` (`"loggedIn": true`), log in again (`claude auth login`, or `/login` in `claude`), then
`send` the prompt again; the session needs no restart. The known lines are each agent's `auth_failures` in
its [provider record](provider-records.md); another agent's expired login ends
by the idle timeout instead.

**The Claude adapter** (`claude-acp`, `@agentclientprotocol/claude-agent-acp`) runs Claude Code through the
Claude Agent SDK, not the interactive `claude` CLI. Whether its usage counts against a Claude subscription plan
login, or needs an API key, depends on the adapter's login and Anthropic's current terms for the Agent SDK.
Check both before relying on a plan login, and note that an `ANTHROPIC_API_KEY` in shed's environment
reaches the agent and is likely to be used instead of the plan.
