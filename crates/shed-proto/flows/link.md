subject: the held link to one machine, on the main host
states: down reconnecting up slow hung
start: down
rest: down up slow

| fact | source | read as |
|---|---|---|
| dialed | ssh | a dial got the hello of the shed there, within 15 s |
| dial_failed | ssh | a dial's ssh exited, or 15 s passed with no hello (the ssh child is killed) |
| refused | ssh | a dial was refused for good: another protocol there, or a host key that is not the pinned one |
| beat | ssh | a server heartbeat arrived within 20 s of the last line |
| steady | ssh | four `beat`s in a row with no `late` or `call_late` between |
| late | ssh | a line arrived, more than 20 s after the last |
| call_late | ssh | a call got no answer within its deadline: 10 s, plus the call's own `timeout` for `wait`, `exec` and `script` |
| silent | ssh | nothing arrived for 30 s |
| closed | ssh | a connection ended or the ssh child exited |
| gave_up | clock | the row's time ran out |

| id | state | fact | next | do | owner | within | or |
|---|---|---|---|---|---|---|---|
| L1 | down | dialed | up | learn | shed | rest | - |
| L2 | down | dial_failed, refused | = | dial | shed | every 60 | - |
| L3 | down | beat, steady, late, call_late, silent, closed, gave_up | = | never | = | = | = |
| L4 | reconnecting | dialed | up | learn | shed | rest | - |
| L5 | reconnecting | dial_failed | = | - | = | = | = |
| L6 | reconnecting | refused | down | dial | shed | every 60 | - |
| L7 | reconnecting | gave_up | down | dial | shed | every 60 | - |
| L8 | reconnecting | beat, steady, late, call_late, silent, closed | = | - | = | = | = |
| L9 | up | beat, steady | = | - | = | = | = |
| L10 | up | late | slow | - | shed | rest | - |
| L11 | up, slow | call_late | slow | fresh | shed | rest | - |
| L12 | up, slow | silent | hung | cut | shed | 5 | cut-stuck |
| L13 | up, slow, hung | closed | reconnecting | dial | shed | 20 | link-down |
| L14 | up, slow, hung | dialed, dial_failed, refused | = | never | = | = | = |
| L15 | slow | steady | up | - | shed | rest | - |
| L16 | slow | beat, late | = | - | = | = | = |
| L17 | hung | beat, steady, late, call_late, silent | = | - | = | = | = |
| L18 | hung | gave_up | reconnecting | dial | shed | 20 | link-down |
| L19 | up, slow | gave_up | = | never | = | = | = |

Actions: `dial` runs one `ssh … shed connect`, cut off at 15 s. `learn` reads the machine's facts, starts the
proxy forward and reads its terminal list for the key flow. `fresh` drops the calls connection the late call
sat on and opens another, so no later call waits behind it. `cut` kills the ssh child and shuts both
connections: every call waiting on the link returns `NO_LINK` at once.

A call for a machine whose link is not `up` or `slow` is answered at once: `NO_LINK: <machine>: link <state>
since <time>`. It never waits for a link.
