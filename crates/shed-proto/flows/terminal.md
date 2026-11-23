subject: a terminal name on one shed; each run of it has an id
states: free starting running asleep done exited gone
start: free
rest: free running asleep done gone

| fact | source | read as |
|---|---|---|
| run | call | `run` names it |
| replace | call | `run` names it, with `replace` |
| input | call | `send`, `keys` or a prompt for it |
| kill | call | `kill` |
| rm | call | `rm` |
| exit | process | the reaper has its exit status |
| lost | process | no status and no such process: a restart or a reboot took it |
| empty | process | nothing is left of its process tree |
| ready | program | it is up: a plain program started, an ACP bridge has its session |
| working | program | it reported a state other than `done` or `hibernated` |
| done | program | it reported `done` |
| slept | program | the bridge reported `hibernated`: the agent is stopped, the session kept |
| expired | record | it ended more than `SHED_PRUNE_AFTER` ago (24 h) |
| gave_up | clock | the row's time ran out |

| id | state | fact | next | do | owner | within | or |
|---|---|---|---|---|---|---|---|
| T1 | free | run, replace | starting | start | program | 180 | start-slow |
| T2 | free | input, kill, rm | = | refuse NO_SUCH_TERMINAL | = | = | = |
| T3 | free | exit, lost, empty, ready, working, done, slept, expired, gave_up | = | never | = | = | = |
| T4 | starting | ready, working | running | - | program | rest | - |
| T5 | starting | done | done | - | caller | rest | - |
| T6 | starting | slept | asleep | - | caller | rest | - |
| T7 | starting | input | = | pass | = | = | = |
| T8 | starting | gave_up | = | - | caller | rest | - |
| T9 | starting, running, asleep, done | exit, lost | exited | end-tree | shed | 10 | tree-left |
| T10 | starting, running, asleep, done | kill | = | end-tree | shed | 10 | kill-stuck |
| T11 | starting, running, asleep, done | run, replace | = | refuse NAME_TAKEN | = | = | = |
| T12 | starting, running, asleep, done | rm | = | refuse STILL_RUNNING | = | = | = |
| T13 | starting, running, asleep, done | empty, expired | = | never | = | = | = |
| T14 | running | input | = | pass | = | = | = |
| T15 | running | ready, working | = | - | = | = | = |
| T16 | running | done | done | - | caller | rest | - |
| T17 | running, done | slept | asleep | - | caller | rest | - |
| T18 | running, asleep, done | gave_up | = | - | caller | rest | - |
| T19 | done | input | running | pass | program | rest | - |
| T20 | done | working | running | - | program | rest | - |
| T21 | done | ready, done | = | - | = | = | = |
| T22 | asleep | input | starting | wake | program | 180 | start-slow |
| T23 | asleep | ready, working | running | - | program | rest | - |
| T24 | asleep | done | done | - | caller | rest | - |
| T25 | asleep | slept | = | - | = | = | = |
| T26 | exited | empty | gone | - | caller | rest | - |
| T27 | exited | rm | free | clear | caller | rest | - |
| T28 | exited | replace | starting | restart | program | 180 | start-slow |
| T29 | exited | run | = | refuse NAME_TAKEN | = | = | = |
| T30 | exited | input | = | refuse NO_SUCH_TERMINAL | = | = | = |
| T31 | exited | kill, ready, working, done, slept, expired | = | - | = | = | = |
| T32 | exited | exit, lost | = | never | = | = | = |
| T33 | exited | gave_up | = | - | caller | rest | - |
| T34 | gone | rm, expired | free | clear | caller | rest | - |
| T35 | gone | replace | starting | restart | program | 180 | start-slow |
| T36 | gone | run | = | refuse NAME_TAKEN | = | = | = |
| T37 | gone | input | = | refuse NO_SUCH_TERMINAL | = | = | = |
| T38 | gone | kill, empty, ready, working, done, slept | = | - | = | = | = |
| T39 | gone | exit, lost | = | never | = | = | = |
| T40 | gone | gave_up | = | never | = | = | = |

Actions: `start` makes the run's id and starts its process. `wake` starts the agent again on its session and
hands it the input. `pass` hands the input to the program. `end-tree` ends what is left of the process tree
(SIGTERM, SIGKILL 2 s later) and child terminals that do not outlive it. `clear` ends what is left at once and
drops the record, its history and its ACP session. `restart` is `clear`, then `start`.

What each state means to a caller:

| state | a process | output readable | name | takes input |
|---|---|---|---|---|
| free | - | - | free | - |
| starting | starting or waking | yes | held | held until ready |
| running | alive; `reported` says working, idle, blocked… | yes | held | yes |
| asleep | the agent stopped by shed, its session kept | yes | held | yes: it wakes |
| done | alive, its turn over | yes | held | yes: a new turn |
| exited | the program ended, exit status known; shed is ending what it left | yes | held | no |
| gone | none: the process table shows nothing of it | yes | held | no |

A terminal lost in a restart goes `exited` (no status) and then `gone` like any other. A `keep` terminal is
`replace`d by the server itself at load, as a shell.
