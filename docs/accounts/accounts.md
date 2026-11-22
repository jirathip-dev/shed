# A run on an account

**This reverses an earlier rule.** shed used to say that it passes its own environment to the agent and "never
handles provider credentials". That no longer holds: shed keeps account folders and setup token files
([store.md](store.md)), and a run names the account it runs on. What still holds: shed stores no OAuth
credential of its own, refreshes nothing, and reads nothing inside an account folder but the record's
`setup_files` and `session_files`. A run without `account` gets shed's environment, as before.

`run`'s `account` (`shed run --acp <id> --account <name>`) is a name from `account.ls` of the agent's provider,
or `default`, the CLI's own folder. On the main host shed then:

- sets the record's `config_dir_var` to the account's folder (for `default`: drops it);
- sets the record's `token_var` to the account's setup token, only while its own login is signed out (a headless
  host whose Keychain is locked);
- drops the record's `scrub` and `cache.api_vars` variables from what the run inherits, so an exported token or
  API key cannot replace the account;
- refuses (`INVALID_PARAMS`) a caller `env` that sets any of those, or the record's proxy variables;
- refuses `ACCOUNT_SETUP` when the folder holds one of the record's `setup_files`, and `NO_ACCOUNT` for an
  unknown account.

Nothing else changes: `meta` reaches `session/new` unchanged and the adapter's command is the same as without
`account`. `get` and `ls` show the account's name as `account`, and the `account` event (`why`: `start`) says
which account a terminal started on. The token stays in memory, like `run`'s `env`: it is in no log, event,
`get`, state file or hibernated session. The bridge keeps the variables, so a wake and a server restart run the
agent on the same account again.

A run on another machine takes no `account` yet (`INVALID_PARAMS`): that comes with the token proxy.

**An expired login** ends the turn at once: the state is `error` with the note `auth: <the agent's line>`
(for `claude-acp`, e.g. `auth: Failed to authenticate: OAuth session expired and could not be refreshed`),
and the `reply` event carries that text with `stop_reason` `auth_error`. To recover, check
`shed account status <provider> <name>`, log in again (`shed account login <provider> <name>`), then
`send` the prompt again; the session needs no restart. The known lines are each agent's `auth_failures` in
its [provider record](../acp/provider-records.md); another agent's expired login ends
by the idle timeout instead.
