subject: one account of one provider, on the main host
states: signed_out signed_in at_limit switched
start: signed_out
rest: signed_out signed_in switched

| fact | source | read as |
|---|---|---|
| in | store | the last status check says signed in, or it holds a token the provider has not rejected |
| out | store | neither; or its keychain item and its file hold different logins |
| limited | store | it has a limit that ends in the future |
| reset | store | it has no limit, or the limit's end has passed |
| moved | runs | no `auto` run is on it |
| gave_up | clock | the row's time ran out |

| id | state | fact | next | do | owner | within | or |
|---|---|---|---|---|---|---|---|
| A1 | signed_out | in | signed_in | - | shed | rest | - |
| A2 | signed_out | out, limited, reset, moved | = | - | = | = | = |
| A3 | signed_in | out | signed_out | - | caller | rest | - |
| A4 | signed_in | limited | at_limit | switch | shed | 30 | no-switch |
| A5 | signed_in | in, reset, moved | = | - | = | = | = |
| A6 | at_limit | moved | switched | - | shed | rest | - |
| A7 | at_limit, switched | reset | signed_in | - | shed | rest | - |
| A8 | at_limit, switched | out | signed_out | - | caller | rest | - |
| A9 | at_limit | in, limited | = | - | = | = | = |
| A10 | at_limit | gave_up | = | - | caller | rest | - |
| A11 | switched | in, limited, moved | = | - | = | = | = |
| A12 | signed_out, signed_in, switched | gave_up | = | never | = | = | = |

Action: `switch` moves each `auto` run on it to the account `auto` would pick now and tells the run to go on.
A run started on it by name is not moved, as today. `no-switch` means every account of that agent is at its
limit.

Whatever sees a limit (a lane's `rate-limited` report, a usage reading) writes it to the store. The flow reads
the store only.
