# Cache share smoke (2026-11-17)

Run on the mini with this branch's `shed` (`serve --detach` in a scratch `--dir` under `env -i` with `HOME`,
`PATH`, `USER` and `TERM`, `SHED_ACP_HIBERNATE_AFTER=20`), `claude-acp` 0.107.2 from the registry (binary),
model `sonnet`, three one-word prompts. `shed get`'s `usage.turn` and `shed top`'s `CACHE`:

| turn | input | cache read | cache write | output | `CACHE` |
| --- | --- | --- | --- | --- | --- |
| 1 | 2 | 11,372 | 18,290 | 6 | 38% |
| 2 | 2 | 13,792 | 17,912 | 8 | 43% |
| 3, the first after `hibernated` and a wake | 2 | 29,679 | 2,657 | 7 | 91% |

Turn 3 woke about 25 s after turn 2, inside the cache's lifetime, so the hibernation kept the cache.

So cooee can talk to Claude through `claude-acp` on the plan login; the `claude -p` stream-json fallback is not
needed.
