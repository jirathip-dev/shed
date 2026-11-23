subject: a machine name in the main host's list
states: removed added up offline draining key_changed
start: removed
rest: removed up offline key_changed

| fact | source | read as |
|---|---|---|
| add | call | `machine.add` with the line it already has, or a new name |
| change | call | `machine.add` whose target, port, key or identity differs from its line |
| rm | call | `machine.rm` |
| listed | list | the file has its line |
| unlisted | list | the file has no line for it |
| link_up | link | its link is `up` or `slow` |
| link_down | link | its link is `down` |
| key_mismatch | link | its link was `refused` over the host key |
| draining | stats | the shed there says it is draining |
| serving | stats | the shed there says it is not |
| gave_up | clock | the row's time ran out |

| id | state | fact | next | do | owner | within | or |
|---|---|---|---|---|---|---|---|
| M1 | removed | add, change, listed | added | link | shed | 120 | never-up |
| M2 | removed | rm, unlisted | = | - | = | = | = |
| M3 | removed | link_up, link_down, key_mismatch, draining, serving, gave_up | = | never | = | = | = |
| M4 | added | link_up | up | - | shed | rest | - |
| M5 | added, up, draining | link_down | offline | - | shed | rest | - |
| M6 | added | gave_up | offline | - | shed | rest | - |
| M7 | added, up, offline, draining, key_changed | add, listed | = | - | = | = | = |
| M8 | added, up, offline, draining, key_changed | change | added | relink | shed | 120 | never-up |
| M9 | added, up, offline, draining, key_changed | rm, unlisted | removed | unlink | caller | rest | - |
| M10 | added, up, offline, draining | key_mismatch | key_changed | hold | caller | rest | - |
| M11 | added | draining, serving | = | - | = | = | = |
| M12 | up | link_up, serving | = | - | = | = | = |
| M13 | up | draining | draining | - | caller | 3600 | drain-slow |
| M14 | offline | link_up | up | - | shed | rest | - |
| M15 | offline | link_down, draining, serving | = | - | = | = | = |
| M16 | draining | serving | up | - | shed | rest | - |
| M17 | draining | link_up, draining | = | - | = | = | = |
| M18 | draining | gave_up | = | - | caller | rest | - |
| M19 | key_changed | link_up, link_down, key_mismatch, draining, serving | = | - | = | = | = |
| M20 | up, offline, key_changed | gave_up | = | never | = | = | = |

Actions: `link` writes its line if the file lacks it and starts its link. `relink` replaces its line and starts
the link again. `unlink` drops its line and its link and records who asked (`by: file` for a hand edit). `hold`
stops dialing and keeps its line.

`removed` is also where a name starts. A removed machine is listed for 24 h with when and by whom. While a
link reconnects, the machine stays `up`: only `link_down` makes it `offline`.
