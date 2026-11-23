subject: one proxy key, on the main host
states: issued in_use ended
start: issued
rest: issued in_use ended

| fact | source | read as |
|---|---|---|
| started | there | its `run` answered with a terminal |
| refused | there | its `run` answered with an error |
| alive | there | the shed there lists its run, in a state other than `gone` |
| gone | there | the shed there answered, and its run is `gone` or not listed |
| unknown | there | no answer about it: the call failed after it was sent, the link is not up, or the machine is removed |
| revoke | call | `proxy.revoke` |
| gave_up | clock | the row's time ran out |

| id | state | fact | next | do | owner | within | or |
|---|---|---|---|---|---|---|---|
| K1 | issued | started, alive | in_use | - | shed | rest | - |
| K2 | issued | refused, gone | ended | - | caller | rest | - |
| K3 | issued | unknown | = | find | shed | 60 | key-unknown |
| K4 | issued, in_use | revoke | ended | - | caller | rest | - |
| K5 | issued | gave_up | = | - | caller | rest | - |
| K6 | in_use | gone | ended | - | caller | rest | - |
| K7 | in_use | started, alive, unknown | = | - | = | = | = |
| K8 | in_use | refused, gave_up | = | never | = | = | = |
| K9 | ended | started, refused, alive, gone, unknown, revoke | = | - | = | = | = |
| K10 | ended | gave_up | = | never | = | = | = |

Action: `find` asks the shed there for the key's run id once its link is up.

A key signs while `issued` and `in_use`. While its `run` call is in flight nothing but that call's answer
speaks for the key; the listing read at link-up speaks for it only afterwards. Every fact is matched by run id,
never by name. So K6 and K4 are the only ways a key in use ends.
