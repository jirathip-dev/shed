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

## Evidence (head a27ac2f7a6e0c2ed86398d8b7d6e0f61fd24da3b, Linux; staging 6347cfe merged in, no conflicts)

- `cargo fmt --all --check` exit 0; `cargo clippy --workspace --all-targets --locked -- -D warnings` exit 0; allow count ≤ 10 exit 0.
- `cargo test --workspace --locked --no-fail-fast`: exit 101. Every unit and integration test passes, `service` included (8 passed). The only failures are the three doc-test targets (`-p shed-core/shed-proto/shed-server --doc`), and those fail because this host has no `rustdoc` binary ("could not execute process `rustdoc`"). CI has rustdoc.
- `scripts/temp-left.sh` exit 0; `scripts/comment-rule.py`, `scripts/hygiene.sh`, `scripts/neutral-core.sh`, `scripts/dep-direction.sh`, `scripts/proto-features.sh` all exit 0; `drover lesson-check` exit 0.
- `python3 -I scripts/changelog.py` and `SHED_BLESS=1 cargo test -p shed-proto --features schema --locked --test protocol_doc` exit 0 and leave no diff.
- `drover arch-check --base origin/staging` exits 1 with a single violation: it runs `just generate`, and `just` is not installed here. CI runs it.
- Not run locally: `cargo deny` and `actionlint` (neither is installed; no dependency or workflow change).

## Repro

Commit 1 (`fd4b192`) holds only `install_force_over_a_unit_without_the_oom_policy_leaves_one_with_it`. It is `#[cfg(target_os = "linux")]` because `shed install` writes a systemd unit only on Linux. On this Linux host:
- At `fd4b192`, `cargo test -p shed --locked --test service install_force` exits 101: the test FAILED and panicked at `crates/shed/tests/service.rs:288` (the `OOMPolicy=continue` assert).
- At head `a27ac2f`, `cargo test -p shed --locked --test service` exits 0 with 8 passed, including that test, `autostart_starts_the_installed_job_and_no_server_of_its_own`, `autostart_without_a_job_starts_a_detached_server` and `doctor_flags_a_server_outside_a_failed_job`.

## Line cap

`src/ net +27` (budget not given). After merging staging 6347cfe, `scripts/line-cap.sh` prints `10969 / cap 11000` (exit 0). The cap was raised in #140, as the owner decided in q1764. The 27 lines are the job start command for each platform, the job start helper, and the doctor row.
