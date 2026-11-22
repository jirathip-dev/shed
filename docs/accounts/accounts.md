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

## The owner's switch: `shed account use`

`shed account use <provider> <name>` (`account.use`) moves the host to another account. A switch happens only
on a limit ([choice.md](choice.md)) or on this verb. It refuses `default` (`INVALID_PARAMS`: `auto` never picks
it) and a named account that is not `ready` (`NO_ACCOUNT`, `ACCOUNT_SIGNED_OUT`, `ACCOUNT_LIMITED`,
`ACCOUNT_SETUP`). Then it:

- records `<name>` as the provider's preferred account in `accounts.json`: `auto` picks it first while it is
  ready and not limited, then the usual order; `account.ls` shows `preferred`, and `shed account use <provider>
  --clear` drops it;
- moves every live ACP or stream lane of the provider that is on `auto` or on `default` (named, or with no
  account) to `<name>`, the slice-4 switch: the bridge stops the agent, copies the session's `session_files`
  from the old folder to the new one, starts it on `<name>` and loads the session. A lane mid-turn or
  `rate-limited` then gets one prompt to go on; an idle lane just waits on the new account. A lane mid tool
  call moves once that call ends. A moved lane is on `auto` with `<name>` current, so its next limit moves it
  again, and its `account` event says `why: owner`.

`default`'s sessions are copied from the record's `default_folder` (Claude Code: `~/.claude`); nothing is ever
written there. Moving a healthy lane costs it its prompt cache: the owner accepts that by running the verb.

What stays, listed in `stayed` with the reason: a lane on another named account, one already on `<name>`, a
terminal that is not an ACP or stream lane (it is never switched), and every lane of a provider without
`session_files`. A run on another machine is not this host's lane and is not listed: re-pointing its key on a
switch is the token proxy's (#265).

**An expired login** ends the turn at once: the state is `error` with the note `auth: <the agent's line>`
(for `claude-acp`, e.g. `auth: Failed to authenticate: OAuth session expired and could not be refreshed`),
and the `reply` event carries that text with `stop_reason` `auth_error`. To recover, check
`shed account status <provider> <name>`, log in again (`shed account login <provider> <name>`), then
`send` the prompt again; the session needs no restart. The known lines are each agent's `auth_failures` in
its [provider record](../acp/provider-records.md); another agent's expired login ends
by the idle timeout instead.
