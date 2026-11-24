subject: this shed itself (#307, slice 2)
states: serving draining
start: serving
rest: serving draining

| fact | source | read as |
|---|---|---|
| drain | call | `machine.drain` |
| lift | call | `machine.drain` with `on: false` |
| gave_up | clock | the row's time ran out |

| id | state | fact | next | do | owner | within | or |
|---|---|---|---|---|---|---|---|
| D1 | serving | drain | draining | end-spares | caller | rest | - |
| D2 | draining | lift | serving | - | caller | rest | - |
| D3 | serving | lift | = | - | = | = | = |
| D4 | draining | drain | = | - | = | = | = |
| D5 | serving, draining | gave_up | = | never | = | = | = |

Action: `end-spares` ends the warm spares. While `draining`, the edge refuses `run` with `DRAINING`, under the
lock that creates a terminal.
