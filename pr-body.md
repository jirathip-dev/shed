Closes #52

## What changed

- **Structured read.** `read` on an ACP terminal takes `events: true` and answers the transcript's events in order, each with its `seq` (its line in the transcript), paged by `before` / `n`. The text `read` is unchanged. CLI: `shed read --events [--json]` (`--before <seq>`, `--lines <n>`); without `--json` it prints the events drawn like the preview, 80 wide and colourless when stdout is no TTY or `NO_COLOR` is set.
- **Tool kind.** `tool_call` carries ACP's `kind` (`read`, `edit`, `execute`, …) when the agent sends one; omitted when absent (the SDK reads an absent kind as `other`, so `other` is omitted too). An update without a kind keeps the call's last one.
- **Readable preview.** The ACP bridge now redraws its own screen from the events on each change, so `attach` and the `shed watch` preview (which mirrors that screen) show: the prompt bold after `›`; one dim line per tool call (`…` running, `✓` done, `✗` failed, kind, the title's first line, cut to the width); approval requests in yellow with their options; the plan as a `[x]`/`[~]`/`[ ]` checklist; the reply's markdown lightly styled (headings bold, lists indented, fenced code indented and dim under its language, inline code highlighted). Wrapped to the terminal width; no colour under `NO_COLOR`. The bridge's own notes ("ACP session ready", errors) stay in the drawing.
- **Agnostic.** Only ACP spec fields are read (`title`, `status`, `kind`, plan `content`/`status`, permission options). An unknown status draws a plain line without a glyph. No per-agent branches.
- Tests: `fake_acp.py --replay FILE` replays recorded session updates; fixtures shaped like Claude Code, Codex and Gemini CLI (`tests/fixtures/acp/`) go through the real bridge, `read --events --json` (snapshot), the renderer at width 80 with colour (snapshot) and `shed read --events` under `NO_COLOR` (snapshot). The test asserts the three give the same block structure, that each multi-line command is whole in the events and one line in the preview, and that the terminal's own screen shows that line.

Decided: the preview is drawn by the bridge rather than by `shed watch` itself, so attach and watch share one drawing and watch needs no ACP branch.

## For humans

An agent terminal's history can now be read as a list of structured steps, so tools like the lane sheet can tell prompts, tool calls and replies apart.
Watching or attaching to an agent terminal now shows a tidy view: the prompt in bold, one short line per tool call with a tick or cross, and the reply's formatting.
This looks the same whichever agent or provider runs in the terminal.

## Tests

At head `21e7081a00174324a3601234020f80b6ec4e8815` (merged `origin/staging` d356469, which raises `line_cap` to 7,500 per q942):

- `cargo fmt --all --check` → 0
- `cargo clippy --workspace --all-targets --locked -- -D warnings` → 0
- `cargo test --workspace --locked --no-fail-fast` → 101: the only failure is `machines::machine_stats_here_and_on_a_machine_and_an_offline_one_answers_within_5_s`, because `sysctl` is not on this lane's PATH (`uptime_s` reads 0); `PATH="$PATH:/usr/sbin" cargo test -p shed --locked --test machines` → 0. This PR touches no stats code. Every other test passes, including the new `acp::recorded_sessions_of_three_agents_read_and_draw_as_the_same_blocks` and `acp::tests::the_preview_draws_an_unknown_status_plain_and_keeps_each_call_on_its_first_line`
- `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --locked` → 0
- `scripts/hygiene.sh` → 0, `scripts/dep-direction.sh` → 0
- `scripts/line-cap.sh` → 0 (`7414 / 7500`)

## Line count

`scripts/line-cap.sh`: 7190 → 7414. `src/ net +224` (no budget stated in the issue): about 120 is the renderer (markdown, wrapping, glyphs, plan, approvals), the rest is the structured read, the tool kind, the bridge redraw and the CLI. The cap was raised to 7,500 for this PR (q942, #54/#55).

## Not verified

- Real Claude Code, Codex and Gemini adapters: the fixtures are shaped after their ACP output, not recorded from live runs.
- The `shed watch` preview by eye; the test checks the terminal's screen through `wait --output`.
