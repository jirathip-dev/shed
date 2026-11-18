Closes #134. Fixes the shed side of drover#1140.

## For humans

When the system runs out of memory and kills one process inside a terminal, shed now keeps running along with every other terminal.
When shed's background service is installed, auto-start brings that service back instead of starting a stray server outside it.
`shed doctor` now warns if a server is running outside the installed service, and tells you how to fix it.

## What changed

- systemd unit (`shed install`, `dist/systemd/shed.service`): `OOMPolicy=continue`. `shed install --force` rewrites an older unit and reloads.
- `--autostart` (`run`, `connect`, every command that connects) with no `--dir` and an installed job file: runs the job's start command (Linux `systemctl --user reset-failed shed.service; systemctl --user start shed.service`, macOS `launchctl kickstart … || launchctl bootstrap …`), then waits for the socket. With no job (or with `--dir`) it starts a detached server as before.
- `shed doctor`: if the job is not loaded but the socket answers, the `loaded` row is `FAIL … a server outside dev.shed.serve answers on <socket>; fix: stop that server, then <start>`, and doctor exits 1.
- `docs/service.md`: new "Out of memory" section. Changelog fragment.

Decided: `--dir` never uses the job, because the job only serves the default socket. `shed connect --autostart` now reuses the same start-and-wait path as the other commands.

## Evidence (head 9077829e7d345ab586ca0fb316e44e5cbb915c4a; tree identical to 278e572, macOS; staging 4241db3 merged in)

- At 278e572: `cargo fmt --all --check` exit 0, `cargo clippy --workspace --all-targets --locked -- -D warnings` exit 0, `scripts/line-cap.sh` exit 0 (`10026 / cap 11000`), `scripts/comment-rule.py` exit 0, changelog regeneration leaves no diff.
- At 278e572: `cargo test --workspace --locked --no-fail-fast` exit 101. 22 test binaries pass, including `service`. The one failure is `machines.rs:632` (`uptime_s` 0 on this host). Staging 4241db3 alone fails the same way here: `cargo test -p shed --test machines --locked machine_stats` exit 101.
- `drover arch-check` exits 1 here only because it calls `just generate` and `just` is not installed. CI runs it.
- At 20838975: `cargo test -p shed --locked --test service`: exit 0, 7 passed. Without the src changes: exit 101 (`autostart_starts_the_installed_job_and_no_server_of_its_own` and `doctor_flags_a_server_outside_a_failed_job` fail).
- Commit 1 (`fd4b192`), the regression test `install_force_over_a_unit_without_the_oom_policy_leaves_one_with_it`, is Linux-only because the unit is only written there. CI's ubuntu job runs it.
- `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, `scripts/comment-rule.py`, `scripts/hygiene.sh`, `scripts/neutral-core.sh`, `cargo doc` with `-D warnings`, `scripts/dep-direction.sh`, `scripts/proto-features.sh`, changelog and protocol-doc regeneration: all exit 0.
- `cargo test --workspace --locked`: exit 101. The one failure is `machines.rs:632` `machine_stats_here_and_on_a_machine_and_an_offline_one_answers_within_5_s`: this host reports `uptime_s` 0. That test uses `--dir` and never reaches the changed path. `scripts/temp-left.sh`: exit 0.
- Not run locally: `just` (not installed; ran its recipes by hand), `cargo deny` (not installed; no dependency change).

## Repro

Commit 1 (`fd4b192`) holds only `install_force_over_a_unit_without_the_oom_policy_leaves_one_with_it`. It is `#[cfg(target_os = "linux")]` because `shed install` writes a systemd unit only on Linux (macOS gets a launchd plist, which has no OOM policy). So there is no cheap way to run it on a macOS host, and CI runs only on PRs to staging/main, so commit 1 alone never had a CI run. On Linux: `git checkout fd4b192 && cargo test -p shed --locked --test service install_force` fails at `assert!(unit.contains("\nOOMPolicy=continue\n"))`. At the head, CI's `stable (ubuntu)` job runs it and passes.

## Line cap

`src/ net +27` (budget not given). After merging staging 4241db3, `scripts/line-cap.sh` prints `10026 / cap 11000`. #140 raised the cap, as the owner decided in q1764. The 27 lines are the job start command for each platform, the job start helper, and the doctor row.
