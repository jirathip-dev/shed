- API-key accounts' tokens and cost this month: each lane's usage on a key-only account adds to the account's
  `spend` (`month`, `tokens`, `cost`) in `account.ls` and `account.status`, priced by the event's own cost,
  else the record's new `[price]` table, else `"unknown"`. The totals survive a restart and start again each
  month, in the machine's local time (docs/accounts/usage.md).
