Closes #127

## For humans

Each agent's data file can now say how long its prompt cache lives on a subscription and on an API key, and which settings mean an API key.
An agent on an API key now goes to sleep once its cache has likely expired (5 minutes for Claude) rather than after 10; subscription agents keep 10 minutes.
`shed get` and `shed top` say whether an idle agent's cache is likely still warm, and switching the model after a turn prints a one-line warning that the cache is lost.
Setting the hibernate-delay variable overrides all of this, as before.

## What changed

- `Provider.cache` (`[cache]`: `plan`, `api` seconds, `api_vars`) in the provider record; `claude.toml` gets 3600 / 300 and `ANTHROPIC_API_KEY`, `ANTHROPIC_AUTH_TOKEN`, `CLAUDE_CODE_USE_BEDROCK|VERTEX|FOUNDRY`. Codex has no data, so it shows no cache line and hibernates as now.
- The bridge sets `session.cache_lifetime` from the record and the lane's environment; the hibernate default is `min(600, lifetime)`. `$SHED_ACP_HIBERNATE_AFTER`, when set, is read exactly as before.
- The server stamps `session.turn_ended` on each non-replay `reply`, keeps it when a new `session` event arrives, and in `get`/`ls`/`top` fills `session.cache_likely` (`warm` / `cold`) while the lane is `done`, `idle` or `hibernated`. `shed top` gains a `CACHE-LIKELY` column.
- `shed model` prints `shed: the model switch drops the prompt cache; the next turn reads the conversation uncached` on stderr, once, only when `turn_ended` is set, then switches as before.
- `docs/acp.md`: a "Prompt cache" section with the source link; the hibernation and provider-record paragraphs name `[cache]`. Generated `docs/protocol.md`, `docs/shed.openrpc.json` and `CHANGELOG.md` are regenerated.

Decided: the plan default stays 600 s (the amendment). Login type comes from the record's `api_vars`, set and not empty. Usage credits on a subscription can't be detected and count as a plan (noted in the docs).

`src/ net +68` (10286 → 10354, cap 11000; the issue states no budget): the new record field, the session fields, the warmth view and the CLI line.

## Evidence (head 192816a4e490d5e7aad41e497eacf1fb05fbd593)

`just` is not installed on this host, so each recipe's commands from the `justfile` were run directly:

- `cargo test --workspace --locked --no-fail-fast`: exit 101. The only failure is `machines::machine_stats_here_and_on_a_machine_and_an_offline_one_answers_within_5_s`. It also fails at base e7b08f4 on this host (`cargo test -p shed --test machines machine_stats`, exit 101), as #126's note says. All other tests pass, including the new ones:
  - `providers::tests::an_api_login_cache_is_likely_cold_after_five_idle_minutes_a_plan_one_after_sixty`
  - `acp::tests::the_hibernate_delay_fits_a_shorter_cache_and_the_variable_overrides_it`
  - `acp::an_api_login_sleeps_once_its_cache_is_likely_cold_and_a_switch_after_a_turn_says_it_drops_it` (a real lane: warm after a turn, the switch line, hibernated at the record's 3 s API lifetime with no variable set, then cold in `get` and `top`)
  - `acp::a_model_switch_is_reported_and_the_next_turn_uses_it` (no line before a turn, no cache line without provider data)
- `cargo fmt --all --check` 0 · `cargo clippy --workspace --all-targets --locked -- -D warnings` 0 · allow count 0 · `scripts/banned.sh` 0 · `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --locked` 0 · `scripts/line-cap.sh` 0 (10354 / 11000) · `scripts/dep-direction.sh && scripts/proto-features.sh` 0 · `scripts/hygiene.sh` 0 · `scripts/neutral-core.sh` 0 · `scripts/comment-rule.py` 0 · `scripts/self-hosted.py` 0 · `drover lesson-check` 0
- `drover arch-check --base e7b08f4`: exit 1. Its only violation is ``just generate` exited 127`` (just missing). I ran the generate steps by hand (`scripts/changelog.py`, `SHED_BLESS=1 cargo test -p shed-proto --features schema --test protocol_doc`) and committed their output.
- Not run here: `cargo deny` and `actionlint` (not installed). `scripts/temp-left.sh` lists `shed-registry-burst-*` from Nov 16 and a `shed-drain-*` folder another lane removed again during the run. These are other runs in the shared TMPDIR, not these tests. CI runs all three.
