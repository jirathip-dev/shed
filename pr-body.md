Closes #87

## For humans

`shed setup` now wires an installed coding agent's own hooks (Claude Code today) so the agent reports when it is working, waiting on a permission, done or failed, with no hand editing.
It only adds its own entries and never reformats the rest of the file, `--dry-run` shows the result first, and `--remove` puts the file back exactly as it was, whatever its layout. Outside shed the hooks do nothing.
`shed doctor` now says per agent whether these hooks are in.

## Decided

- **Implement, not close.** The hooks run only when an agent is started inside shed, and setup touches nothing but its own marked entries.
- **Hook command:** `[ -z "$SHED" ] || shed state <state>`. The shell guard keeps it a no-op that exits 0 outside shed. An earlier `shed state --hook` flag was dropped to stay inside the line cap.
- **Text edit, not re-serialization (round 2 fix).** Setup inserts shed's entries as compact JSON right after the `{`/`[` that opens their list and touches no other byte; `--remove` deletes exactly that inserted text. A whole new `hooks`/event member is written as `"E":[ {…}]` (space after the bracket), a form an insertion into the user's own `"E":[]` can never produce, so remove never takes out a list the user wrote. Shed's entry is recognised by exact equality with its group, so a user hook that merely mentions `shed state` is never touched (round 1 non-blocking findings).
- **Claude Code table** (`providers/claude.rs`, `~/.claude/settings.json`):
  - `UserPromptSubmit` and `PostToolUse` → working
  - `PermissionRequest` → blocked
  - `Stop` → done
  - `StopFailure` → error

  `PostToolUse` puts the state back to working after an approval. `Notification` is not used, because its idle prompt would turn done into blocked.
- **Codex:** there is no Codex provider module, so it is out of scope (scope 1: "the installed agents shed has a provider module for").
- **Installed** means the agent's config folder exists. When nothing is installed, `shed setup` exits 1 with a message. An unknown agent name is an error.
- **serde_json `raw_value`** (a feature of an existing crate, no new crate, lock unchanged) gives each list's position in the original text.
- **Absent file (round 3 fix):** setup writes a missing config as `{\n"hooks":{ …}\n}\n`, a text no insertion into an existing file can produce (insertion always lands right after the `{`). `--remove` deletes the file only while it is exactly that text; any other file, including a user's own `{}\n`, is only stripped, so it comes back byte for byte.
- **Doctor:** the hooks lines are informational (`ok`, or `--` with the fix) and don't change doctor's exit code, because hooks are opt-in. A config setup cannot read (e.g. not JSON) prints a `FAIL  hooks` line with the error and fails doctor (round 2 non-blocking finding).

## Line cap

`src/ net +184` (no budget given): the text edit plus setup, the provider tables and the doctor lines. With #106 merged (q1551/q1556, cap raised to 10000) the repo is at 7979 / cap 10000; `just cap` passes.

## Evidence (head 3a3a3ca6e9d446935069eb16c45095b1ac3da996)

- `just fmt clippy banned doc cap deps hygiene neutral comments self-hosted arch lessons`: each exit 0 (`scripts/neutral-core.sh` passes; cap 7979 / 10000).
- `cargo test -p shed-server --locked hooks::`: exit 0. The same test with the new `{}\n` fixture on the round-2 code: exit 101 (the config is gone after remove).
- `cargo test -p shed --locked --test service`: exit 0 (3 passed).
- `just test`: exit 101, only `machine_stats_here_and_on_a_machine_and_an_offline_one_answers_within_5_s` (fails on the unchanged base on this host too).
- Reviewer's repro through the real binary: `printf '{}\n' > $H/.claude/settings.json; HOME=$H shed setup; HOME=$H shed setup --remove; cmp orig $H/.claude/settings.json`: `cmp` exit 0. Absent config: setup then remove leaves `.claude/` empty again. `{bad` config: `shed doctor` prints `FAIL  hooks    …settings.json: not JSON objects: key must be a string at line 1 column 2`.
- `just temp-left`: exit 1 only on other runs' folders in the shared TMPDIR (none `shed-setup*`); with a fresh `TMPDIR`, `scripts/temp-left.sh` after `cargo test --workspace`: exit 0.
- `just deny` / `just actionlint`: not run (cargo-deny and actionlint not installed here); CI runs them.

Tests:

- `hooks::tests::setup_adds_its_hooks_once_and_remove_restores_the_file_byte_for_byte` (fake provider): absent config → setup → remove → absent; then four non-canonical configs (the reviewer's mixed-indent file, a user `Stop` group with tabs plus an empty `Other` list and odd spacing, compact `"Prompt":[]`/`"Stop": [ ]` lists, an empty `hooks` object with leading spaces, a user's `{}\n` and `{\n\n}\n`). For each: dry run writes nothing, setup equals the preview and holds both hooks, a second run changes nothing, `--remove` gives the original back byte for byte. The first three fail on the round-1 code.
- `setup_hooks_report_nothing_outside_shed_and_doctor_lists_them`: real `shed setup` with a temporary HOME, the installed hook run through `sh` without `$SHED` exits 0 with no output; doctor `--` before, `ok` after.
