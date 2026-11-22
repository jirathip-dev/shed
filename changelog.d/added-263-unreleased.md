- `shed run --acp <id> --account <name>` (`run`'s `account`) runs the agent on one of shed's accounts: its
  folder set, its setup token too while its own login is signed out, and none of the record's `scrub` or API-key
  variables inherited from shed; a caller `env` that sets one of them is refused. `get` shows the `account`, an
  `account` event says which one it started on, and the token is in no log, event, state file or hibernated
  session. This reverses "shed never handles provider credentials" (docs/accounts/accounts.md). Protocol 5.
