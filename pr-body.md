Closes #73

## For humans

The server no longer rewrites and syncs its whole state file on every agent turn: it saves at most twice a second, in compact form, without holding up other requests, and right away when it stops.
Agents' slash-command and model lists are no longer saved (the agent sends them again), so the file is a fraction of its old size.
Exited terminals drop off the list a day after they end (`SHED_PRUNE_AFTER` seconds to change it); `run`, `kill` and `rm` still answer only once the change is on disk.

## What changed

- `registry.rs`: the saver is a tokio task (`keep_saved`): a dirty `Notify`, snapshot under the lock, `write_state` in `spawn_blocking` outside it, then a 500 ms wait. `run`/`kill`/`rm` mark the change urgent: it cuts the wait, and their reply still waits until it is on disk (the #72 guarantee, now for those three only). `state`, session and refresh changes are debounced and their replies don't wait.
- Compact JSON (`to_writer`); the snapshot clears `session.commands` and `session.models` (kept in memory).
- `exited_at` is saved beside each terminal via a private `Saved` wrapper (`#[serde(flatten)]`), so the wire `TerminalInfo` is unchanged. Terminals already exited in an old file get the load time.
- `prune` runs on the stats tick (`record`, every 60 s) and drops exited terminals older than `$SHED_PRUNE_AFTER` (default 86 400 s). It deletes no files; the existing orphan sweep at the next start still removes them.
- Shutdown: `close()` writes one last snapshot and stops the saver *before* terminals are ended, so the file shows what was running and `keep` terminals come back as shells, as `docs/service.md` says.

## Staging build fix (round 2, head 4f986aea992c4615fc267fe4258cb868a662cfad)

CI `rules` was red on this PR and on `staging` itself: its `install herdsman` step failed (`could not find \`herdsman\` in .../herdsman?branch=staging`) because the herdsman repo renamed its crate and binary to `drover` on staging at 02:02 UTC (upstream 9dbeef9ff, #932). The fix pins the install to `--rev 1534b233d711fd14169d82ebd5710b027e1b1d4c`, the staging commit just before that rename. Its crate is still named `herdsman`, so the justfile's `herdsman arch-check` / `lesson-check` calls and `.herdsman/` paths stay unchanged. Moving to `drover` is a separate migration.

At 4f986aea992c4615fc267fe4258cb868a662cfad (diff vs 83f6f9f: `.github/workflows/ci.yml` only):
- `mise exec actionlint@1.7.12 -- actionlint`: exit 0
- `herdsman arch-check`: exit 0 (167 files, 0 violations)
- `herdsman lesson-check`: exit 0

## Evidence (src unchanged since 83f6f9fcdf04a1fc0a1f776c256bc5e1d3256e43, where it ran)

- `just fmt clippy comments arch cap`: exit 0 (cap `8466 / cap 10000`)
- `just fmt doc hygiene neutral deps lessons`: exit 0
- `just banned self-hosted`: exit 0
- `PATH=$PATH:/usr/sbin:/sbin TMPDIR=<fresh private dir> just test temp-left`: exit 0
- New tests (crates/shed-server/src/registry.rs):
  - AC2 `a_thousand_changes_in_a_second_write_at_most_three_times_and_end_on_disk`: paused tokio clock, 1,000 `state` RPCs 1 ms apart, real writes counted from the file (mtime + bytes). Asserts 1–3 writes, and that the file holds the last change 500 ms after it. Mutation check: with the 500 ms wait set to zero, it sees 1000 writes and fails.
  - AC3 `seventy_acp_terminals_save_under_300_kb_without_commands_or_models`: a >1 MB old-style file of 70 ACP terminals loads and saves under 300 KB with `{id, model}` kept; in-memory `commands` are kept.
  - AC4 `an_exited_terminal_past_the_limit_is_pruned_and_a_running_one_kept`.
  - AC5: writes stay atomic (tmp, fsync, rename, unchanged), and the AC2 test proves the file trails the state by at most 500 ms.
- `just deny` was not run: the tool is not installed on this host (no dependency changes). actionlint: see the staging build fix above.

Environment notes: this lane's PATH lacks `/usr/sbin`, so `sysctl` was missing and uptime read 0, failing `machine_stats…` (it passes with `/usr/sbin` on PATH). The shared `$TMPDIR` holds hundreds of other lanes' `shed-*` folders, so `temp-left` was run with a fresh private TMPDIR. Under load average ~33, `lock.rs killing_a_program_that_ignores_sigterm_holds_up_no_list` (`ls` < 100 ms) is flaky: it failed 4/5 on `origin/staging` (up to 919 ms) and 1/5 on this branch (107 ms).

src/ net +87 (no budget stated in the issue): the save task, the `exited_at` wrapper and prune.

<!-- codesmith:footer -->
---
<a href="https://backend.blacksmith.sh/track/enable-autofix?expires=1797128087&installation_model_id=428821&pr_number=111&ref=codesmith_pr_footer&repository=jirathip-dev%2Fshed&return_to=https%3A%2F%2Fgithub.com%2Fjirathip-dev%2Fshed%2Fpull%2F111&signature=ca73843dfdac9579372079e717968384d90affc7cc4af4d0df6026600e366577"><picture><source media="(prefers-color-scheme: dark)" srcset="https://pr-comments-assets.blacksmith.sh/codesmith/autofix-with-codesmith-dark.svg"><source media="(prefers-color-scheme: light)" srcset="https://pr-comments-assets.blacksmith.sh/codesmith/autofix-with-codesmith-light.svg"><img alt="Autofix with [code]smith" src="https://pr-comments-assets.blacksmith.sh/codesmith/autofix-with-codesmith-dark-3x.png" width="190" height="28"></picture></a>
<sup><a href="https://app.blacksmith.sh/jirathip-dev/codesmith/shed/pr/111?autoLogin=true&ref=codesmith_pr_footer">Open in [code]smith</a> · Need help on this PR? Tag <code>@codesmith-bot</code> with what you need.</sup>

<!-- codesmith:autofix:disabled -->
<!-- /codesmith:footer -->

## Why (round 1)

Decided: run, kill and rm replies still wait for disk; state, session and refresh saves are debounced to 500 ms
Because: "at most one save per 500 ms, and an immediate save on shutdown"
Rejected: every reply waits for disk as #72 did — each bridge state RPC would force a write and AC2 could not hold
Decided: the final save runs before terminals are ended at shutdown
Because: crates/shed-server/src/lib.rs:316
Rejected: saving after the kills — keep terminals would be saved exited and never come back as shells
Decided: exited_at lives in a private flattened Saved wrapper, so the wire TerminalInfo is unchanged
Because: crates/shed-server/src/registry.rs:65
Decided: prune keeps history and transcript files; the existing orphan sweep at the next start removes them
Because: "with their transcripts kept on disk under their existing cap"
Check: the burst test counts real file writes on a paused clock; with the wait set to zero it sees 1000 writes
Owner: the server saves its state at most twice a second and drops exited terminals a day after they end

<sub>note on 83f6f9f · git log --notes=why</sub>

