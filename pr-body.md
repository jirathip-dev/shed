Closes #131

## For humans

The shed server now uses three threads per terminal instead of five, so 200 idle terminals take about 600 threads instead of 1000 and a little less memory.
Nothing changes in how terminals behave: input, the stuck state and exit codes work as before.

## What changed

- The reaper and the input writer are tasks, not two threads per terminal. They run on the caller's multi-thread tokio runtime (serve's), else on a shared one-worker runtime in shed-core (tests, current-thread runtimes). The reaper re-checks its child on every SIGCHLD; the input writer writes one send at a time through tokio's pipe Sender on a dup of the master, made non-blocking only for that send (`stuck` unchanged).
- The PTY reader, feeder and log writer stay threads; the reader polls only if a read lands in a writer's non-blocking window. Terminal replies (DA, cursor reports) go through the input writer's queue.
- `scripts/bench-idle.sh` and CI job `idle-terminals` (ubuntu and macOS, `ulimit -Sn 256`): 200 idle terminals start, are listed, and serve has at most 3 threads per terminal plus 40.
- `docs/bench.md`: threads and RSS for 22 and 200 idle terminals, before and after.

Decided: a current-thread runtime is never used, since `Terminal::write` and `kill` block their thread waiting on these tasks (registry's paused-clock tests hung). serve's multi-thread runtime is used: every blocking call there goes through `spawn_blocking` or its own thread. Fix round 1: the Linux `exited_sessions_release_their_fds` failure was the private runtime's epoll, waker and signal fds, opened at the first terminal after the test's fd baseline; sharing serve's runtime opens none. If SIGCHLD can't be watched, the reaper now waits on a blocking task instead of recording a running child as exited. AsyncFd is not used: its registration is `unsafe` in tokio 1.54.

## Evidence

Fix round 2, head 2658afd31514233d2d8aa6d144b5cc57f97020e4 (staging with the cap raise #140 merged in; macOS):
- `scripts/line-cap.sh`: exit 0 (10039 / cap 11000).
- `PATH="$PATH:/usr/sbin:/sbin" cargo test --workspace --locked`: exit 0 (193 passed). Without `/usr/sbin` on PATH, `machine_stats_..._5_s` fails only because this lane's PATH lacks `sysctl` (uptime reads 0), which is an environment problem, not a code one.
- `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, `scripts/comment-rule.py`, `cargo doc` (-D warnings): exit 0.
- Generate steps (`scripts/changelog.py`, `SHED_BLESS=1 cargo test -p shed-proto --features schema --test protocol_doc`): exit 0, no diff. `drover arch-check` exits 1 here only because it calls `just`, which isn't installed.
- Not run locally: `cargo deny`, `just`, actionlint (not installed); CI runs them.

Round 0 (head a9976f92df33430e72643b878526cb5e5f770863, macOS M-series, load 20 to 47); thread counts are unchanged in shape since serve's runtime has no fewer threads than the private one:

| idle terminals | base threads | head threads | base RSS | head RSS |
|---|---|---|---|---|
| 22 | 116 | 73 | 8.7 MB | 8.0 MB |
| 200 | 1007 | 608 | 37.5 MB | 31.9 MB |

- `scripts/bench-idle.sh <shed> 200` under `ulimit -Sn 256`: head exit 0, base c4e8194 exit 1 (1007 threads > 640).
- `cargo test --workspace --locked --no-fail-fast`: exit 101; the only failure is `machine_stats_here_and_on_a_machine_and_an_offline_one_answers_within_5_s` (`uptime_s` 0 on this host), which fails identically at base c4e8194 (exit 101). Input tests (#83) and the reliability suite pass.
- `cargo test -p shed-core --locked`: exit 0. `cargo clippy --workspace --all-targets --locked -- -D warnings`: exit 0. `cargo fmt --all --check`, `cargo doc` (-D warnings), banned, deps, hygiene, neutral, comments, self-hosted: exit 0.
- `scripts/bench-drain.sh base .` locally: plain 111%, colour 100%, redraw 103% at load up to 47 (the raw-read ceiling swung 13 to 123 MB/s in the same run); CI's bench-drain is the gate. An earlier version that left the master non-blocking was 115 to 131% and was reverted.
- Not run locally: `cargo deny` and `just` (not installed), actionlint.

## Line cap

src/ net +40 (budget none stated): the runtime choice, async reaper with its blocking fallback, and async writer. Per owner decision q1767 the cap was raised to 11000 in #140; at 2658afd the total is 10039 / 11000.

## Why (round 2)

Decided: the reaper and input writer run on the caller's multi-thread tokio runtime (serve's), else on a private one-worker runtime
Because: crates/shed-core/src/terminal.rs:505
Because: crates/shed/tests/reliability.rs:418
Decided: the Linux fd failure was the private runtime's epoll, waker and signal fds opened lazily after the test's baseline, not a per-session leak
Rejected: any current runtime — a current-thread test runtime that blocks in kill can never run the reaper (registry tests hung)
Rejected: changing the fd test — the spec says "the reliability suite pass unchanged"
Decided: if SIGCHLD cannot be watched the reaper waits for the child on a blocking task instead of recording it as exited
Because: crates/shed-core/src/terminal.rs:512
Decided: the cap overrun waits for the owner's raise in one orchestrator-filed issue
Because: q1767
Check: exited_sessions_release_their_fds passes on the Linux CI jobs (stable ubuntu, msrv) at this head
Check: bench-drain on CI stays within 110%
Owner: fixes the leftover file-handle failure on Linux by sharing the server's background runtime instead of starting a second one

<sub>note on ebf10b3 · git log --notes=why</sub>

