# `auto` and the switch on a limit

`run`'s `account` may be `auto` (`shed run --acp <id> --account auto`). shed then picks among the provider's
named accounts, never `default` (the owner's own folder: a caller that wants it names it). A candidate is
`ready` and its folder holds no setup; the preferred one (`shed account use`) wins, then the one with the most
headroom, then the one with fewer runs, then
by name. Headroom is 100 less the used percentage of its fullest usage window; a window past its reset reads
as 0 % used, and an account never read has all of it. `account.ls` lists each provider's accounts in that
order, after `default`. With no candidate the run is refused: `NO_ACCOUNT` without a named account,
`ACCOUNT_LIMITED` (the first reset in the message) when one is limited, else `ACCOUNT_SIGNED_OUT`.

`get` shows the account the lane is on now, and the `account` event (`why: start`) says which one it started
on.

When a lane on `auto` hits a limit (the record's limit line, or the stream driver's limit event):

1. it reports `rate-limited`, and shed marks its account limited ([usage.md](usage.md));
2. shed picks again among the other accounts; with none left the lane stays `rate-limited`, its note the first
   reset when every account is limited;
3. otherwise, once the turn has ended, the bridge stops the agent, copies the session's files (the record's
   `session_files`, e.g. `projects/*/{session}.jsonl` for Claude Code) from the old account's folder to the
   new one's, starts the agent with the new account's variables and loads the session, as a wake from
   hibernation does;
4. it sends one prompt to go on, and emits `account` with `was` (the old account) and `why: limit`.

A `rate-limited` lane on `auto` picks again when its next turn starts: if another account is ready the turn
starts there, else on the same account. shed starts no turn by itself, and moves a lane only on a limit or on
`account use` ([accounts.md](accounts.md)). An agent whose record has no `session_files` never moves: its lane
stays `rate-limited`. A fallback to another agent (Claude, then Codex) is the caller's.

The live check that a Claude Code session copied between two folders loads is
`crates/shed-accounts/tests/live_session.rs` (ignored by default; its header has the command).
