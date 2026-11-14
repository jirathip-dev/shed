Closes #81

## For humans

Programs that speak the Program Status Protocol now show everything they report in shed: why they are blocked, how far along they are, their name, a title and a message, and separate statuses for their sub-tasks.
shed answers the protocol's "are you there?" question, so agents that wait for that answer before reporting now report too.
A program that exits while working or blocked no longer looks stuck. A done or error status stays after it exits.
`shed wait --until blocked --kind question` waits only for a question, not for a permission prompt.

## Spec

The canonical spec is Rex's Program Status Protocol, revision 0.4 (2026-10-10). It is noted in `docs/protocol.md` under `state`. Where it differs from the issue's list, the spec wins:
- The probe reply is `ESC ] 7501 ; ?:states=…:kinds=… ST`. shed lists `clear` too, and replies with the same terminator as the query.
- `clear` with no `id` drops every record. With an `id`, it drops that record and every record under it.
- `msg` and `title` are base64. Bad base64, a control character in the text, or an over-limit value drops the whole report. A bad `state` or `id` drops it too. A bad `kind`, `progress` or `app` is treated as absent. Unknown keys and malformed pairs are skipped.

Not done: the spec's `app` inheritance from parent records, and dropping records at OSC 133 prompts. Neither is in the issue's scope or acceptance criteria.

## What changed

- **Parser** (`shed-core` `screen.rs`): reads the full `state=<s>[:k=v]…` form and answers `?`. shed's `state=<s>;note=<n>` form keeps working.
- **Records** (`terminal.rs`): the root report plus one record per id, at most 256. On exit, `working` and `blocked` are dropped, including reports that arrive after the exit is seen.
- **Protocol**: `Reported` gains `id`, `kind`, `progress`, `app`, `title` and `msg`. `TerminalInfo` gains `records`. `wait` gains `kind` (CLI: `--kind`). A `state` event is emitted for each new or changed record.

## Evidence (head 92be5e99f6ef43a437b95170d74598bd583b1d07)

- Round 9: the review FAIL was CI `stable (macos)` red at 130fb11 on `serve_raises_a_lowered_soft_limit_so_many_sessions_spawn` (`pty: Device not configured`, os error 6). That is macOS running out of PTYs (`kern.tty.ptmx_max` 511), not a leak from this branch. That test and `exited_sessions_release_their_fds` each open hundreds of PTYs in parallel, and the same tests pass on staging CI. `cargo test -p shed --test reliability` exits 0 twice at this head on a host that already has 120–140 PTYs open, and no shed server or terminal from the test run is left afterward. A full workspace run here hit the same ENXIO in those two tests while other lanes held PTYs. Merged staging 4de49d0 (#87, #88); the doc sentence about terminals keeps both this branch's OSC 7501 fields and #88's `usage`.
- Round 9 gates at 92be5e99f6ef43a437b95170d74598bd583b1d07: `just fmt clippy banned doc cap deps hygiene neutral comments self-hosted arch lessons` each exit 0; `just cap` 9396 / 10000. `TMPDIR=<fresh short dir> cargo test --workspace --no-fail-fast` exit 101 from `machine_stats_..._within_5_s` only (the known host `uptime_s:0` failure); `just temp-left` exit 0. `just deny` and `just actionlint`: the tools are not on this host, so CI runs them.
- Earlier rounds, at 130fb11:
- Round 8: CI `rules` was red at 77ae802 because `cargo install` could no longer find `herdsman` on its `staging` branch (renamed to drover). Merged staging bf677db, which pins that install to the commit before the rename (4f986ae, staging build fix), plus #73 and #83. The merge was clean; #73's save test built a `Reported` by hand, so it now fills the new fields with `..Reported::default()`.
- Round 8 gates at 130fb11612ab2008177cad3a220a887ae1f6ec67: `just fmt clippy banned doc cap deps hygiene neutral comments self-hosted arch lessons` each exit 0; `just cap` 9087 / 10000. `TMPDIR=<fresh short dir> cargo test --workspace --no-fail-fast` exit 101 from `machine_stats_..._within_5_s` only (the known host `uptime_s:0` failure); `just temp-left` exit 0. `just deny` and `just actionlint`: the tools are not on this host, so CI runs them.
- Earlier rounds, at 77ae802:
- Round 7: merged staging 4254d78 (#84: parent and child terminals, `wait --children`, notes to the parent). Resolved `registry.rs`: `refresh` keeps this branch's per-record `state` events and staging's notes, sending a note to the parent when the root report turns `blocked` or `error`; `exited` copies the report and records and returns staging's note; the ACP note's `working` report fills the new fields with defaults. Resolved the `wait` description and the terminal sentence in `lib.rs` and the doc test so both sides' text is kept. `docs/protocol.md` and `docs/shed.openrpc.json` were re-blessed (`SHED_BLESS=1 cargo test -p shed-proto`); next to round 6 they only add staging's `parent` text and examples.
- Round 7 gates at this head: `just fmt clippy banned doc cap deps hygiene neutral comments self-hosted arch lessons` each exit 0; `just cap` 8821 / 10000. `TMPDIR=<fresh short dir> cargo test --workspace --locked --no-fail-fast` exit 101: `machine_stats_..._within_5_s` (the known host `uptime_s:0` failure) and once `killing_a_program_that_ignores_sigterm_holds_up_no_list` (`ls took 155ms`, limit 100 ms, under full-suite load). That lock test then passed 5/5 alone at head and at staging, and the full `--test lock` suite passed 4/4 at each, run alternately (one earlier head run missed by 0.4 ms). `just temp-left` exit 0. `just deny` and `just actionlint`: the tools are not on this host.
- Earlier rounds, at 010a04a:
- Round 6: merged staging a2befd9 (#80: the screen is fed on its own thread, `feed_loop`). Resolved `terminal.rs`: `read_loop` is staging's; `feed_loop` applies every OSC 7501 report from `take_reports` (the per-id records) where it used to take one report, and writes the probe reply with the screen's other replies.
- Round 6 gates at this head: `just fmt clippy banned doc cap deps hygiene neutral comments self-hosted arch lessons` each exit 0; `just cap` 8572 / 10000. `TMPDIR=<fresh dir> cargo test --workspace --no-fail-fast` exit 101 from `machine_stats_..._within_5_s` only (the host `uptime_s:0` failure below); `scripts/temp-left.sh <that dir>` exit 0. `just deny` and `just actionlint`: the tools are not on this host.
- Earlier rounds, at d17f265:
- Round 5: merged staging afc2107 (#76: a reaper thread owns the child, no `closed` flag). Resolved `terminal.rs`: the reaper and the PTY reader each end the report (`working` and `blocked` dropped) when they see the exit; `try_wait` and `kill` are staging's; the `stuck` report fills the new fields with defaults. Resolved `docs/protocol.md` and its doc test: one sentence carries both the OSC 7501 fields and `stuck`.
- Round 5 gates at this head: `just fmt clippy banned doc cap deps hygiene neutral comments self-hosted arch lessons` each exit 0; `just cap` 8117 / 10000. `cargo test --workspace --locked --no-fail-fast` exit 101 from `machine_stats_..._within_5_s` only (the host `uptime_s:0` failure below); every other test passes, `program_status_osc_7501_...` and `osc_7501_state_events_...` included. `just deny` and `just actionlint`: the tools are not on this host. This run left no temp folders; the `shed-nofile-*-55816` folders seen during it belong to another lane's still-running test binary.
- Earlier rounds, at 608f5c5:
- Round 4: merged staging b5a69b2 (the #106 cap raise to 10000). It merged cleanly and touched only the cap files. The #106 block is dropped (`herdsman queue berth unblock 81`).
- Round 3 fixes:
  - Merged staging 719738b (#86 OpenRPC). `Reported` derives `Default` and `JsonSchema`. `docs/shed.openrpc.json` was re-blessed; the diff only adds the new `Reported` fields and `wait`'s `kind`, and updates the `state` and `wait` descriptions.
  - Merged staging 62e565a. In the screen, the conflicts kept #85's `answered` and `relay` next to this PR's report list. The probe reply now goes through the screen's `reply`, so the relay strips the answered probe like other queries. The `program-status.txt` golden no longer wraps the probe line, because #85 widened the default to 200 columns. That is the only line that changed.
  - Fan-out bench (CI `bench-fanout` was at 115%): `Reported` grew from 72 to 200 bytes, which grew `EventKind` from 120 to 200 bytes and every event in the ring with it. `EventKind::State` now holds a `Box<Reported>`, so `EventKind` is back to 120 bytes, the same as base. The JSON is unchanged.
  - `BENCH_TARGET=$HERDSMAN_SCRATCH_TARGET/target scripts/bench-fanout.sh <staging checkout> .`: exit 0, head/base 87%. Before the box it was 105% here; this host is noisy, so CI's Linux run is the judge.
- `just fmt clippy banned doc arch comments neutral deps hygiene lessons`: each exit 0 at this head.
- `just cap`: exit 0, at 7994 / 10000 (this PR +199).
- `cargo test --workspace --locked --no-fail-fast`: exit 101, from one test only: `machine_stats_here_and_on_a_machine_and_an_offline_one_answers_within_5_s` (`uptime_s:0` on this host). It fails the same way at staging 62e565a: `cargo test -p shed --test machines machine_stats` exits 101. Every other test passes. An earlier run at this head, made while the disk was filling, also failed `a_child_allocating_50_mb_fires_a_resources_event_and_tops_the_list` and `serve_raises_a_lowered_soft_limit_so_many_sessions_spawn`. With space freed, `cargo test -p shed --test cli a_child_allocating_50_mb` and `cargo test -p shed --test reliability serve_raises_a_lowered_soft_limit` each exit 0, and both pass in the full run above.
- `just deny` and `just actionlint`: these tools are not installed on this host, so CI's `rules` job runs them. `just temp-left` lists `shed-watch-*` folders in the shared TMPDIR dated Nov 10–11, from before this round; none come from this run.
- Round 2 kept: `kill` drops `working` and `blocked` reports; the `kill` and `rm` goldens show it.

Tests:
- The PTY fixture is `cli.rs` `program_status_osc_7501_keys_ids_probe_clear_and_lifetime`, golden `tests/golden/program-status.txt`. It covers the probe reply read back by the program, every key, three ids, `clear` by id and in full, `wait --kind`, and an exit while `working` versus an exit while `done`.
- The events test is `server.rs` `osc_7501_state_events_carry_the_keys_and_each_id`.
- The parser unit tests are in `screen.rs`: every state, every key, garbled reports, and the probe reply with both terminators.

src/ net +199 (no budget given): the parser and base64 decoder in core (no new dependency), the per-id records, and the protocol fields.

## Why (round 1)

Decided: the canonical spec is Rex's Program Status Protocol revision 0.4; where it differs from the issue, it wins
Because: "Where it differs from the list above, the spec wins and this issue is amended."
Decided: the probe reply is ?:states=…:kinds=… in the query's own terminator (BEL or ST)
Because: crates/shed-core/src/screen.rs:668
Decided: a report with an id lives in the terminal's records (max 256); clear drops that id and those under it, plain clear drops all
Because: "Reports with an `id` are kept per id. The terminal's own state is the root's; `get` lists the others."
Decided: working and blocked are dropped once try_wait sees the exit or the PTY closes; late reports after that are pruned too
Because: crates/shed-core/src/terminal.rs:66
Decided: base64 for title and msg is a 20-line decoder in core, not the base64 crate
Rejected: adding base64 as a direct dependency — new dependencies need the owner's approval
Decided: refresh calls try_wait before reading the report, so a get after exit never shows a dropped working
Because: crates/shed-server/src/registry.rs:250
Check: program-status.txt golden: probe reply, every key, three ids, clear, wait --kind, exit working vs done
Check: machine_stats_..._within_5_s fails identically at base 05f4d50 on this host; every other test passes
Owner: agents speaking the status protocol now show why they are blocked, progress and sub-tasks, and stale working states vanish on exit

<sub>note on 56080d8 · git log --notes=why</sub>

## Why (round 2)

Decided: Terminal::kill ends the report after reaping the child, so kill drops working and blocked like try_wait does
Because: crates/shed-core/src/terminal.rs:361
Decided: exited copies the terminal's report and records, so the kill reply carries the dropped state at once
Because: crates/shed-server/src/registry.rs:286
Decided: the kill and rm goldens now expect no reported after a kill; the old golden recorded the stale working
Because: "On exit, `working` and `blocked` are dropped and `done` and `error` are kept."
Check: cargo test -p shed --test server run_ls_read_kill passes; the kill.jsonl and rm.jsonl diffs drop only reported
Check: machine_stats_..._within_5_s re-run at base 05f4d50 this round: same uptime_s 0 failure, exit 101
Rejected: only re-blessing the goldens — the kill reply could still race the report, so the fix ends it in kill itself
Owner: killing a program that reported working or blocked no longer leaves that stale state on it

<sub>note on 3d8a9db · git log --notes=why</sub>

## Why (round 3)

Decided: merged origin/staging b5a69b2 (the #106 cap raise); it touched only the cap files
Because: ci/cap.toml:3
Rejected: rebasing onto staging — lane rules forbid history rewrites and force-pushes
Decided: dropped #81's block on #106 now that the raise is on staging
Because: ci/cap.toml:3
Check: just cap exits 0 at 7994 / 10000
Check: the 50 MB resources and lowered-soft-limit tests pass alone and in the full run; their earlier failures came while the disk was full
Check: machine_stats_..._within_5_s is the only failing test, the same host uptime 0 failure as at staging
Owner: the cap no longer blocks this PR; it is ready for review

<sub>note on 608f5c5 · git log --notes=why</sub>

## Why (round 4)

Decided: merged origin/staging bf677db, whose 4f986ae pins the herdsman install that broke CI's rules job
Because: .github/workflows/ci.yml:73
Rejected: editing ci.yml on this branch — staging already carries the fix, so the merge brings it
Decided: #73's save test builds Reported with ..Reported::default() for the new fields
Because: crates/shed-server/src/registry.rs:1111
Check: the rules job's herdsman install succeeds and the job is green at this head
Check: TMPDIR=<fresh short dir> cargo test --workspace --no-fail-fast fails only machine_stats_..._within_5_s, the known host uptime 0 failure
Owner: the branch is current with staging again, which also fixes the broken CI tool install

<sub>note on 130fb11 · git log --notes=why</sub>

