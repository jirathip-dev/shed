# Trial: jcode, Pi and OpenCode as ACP agents on a DeepSeek key (#375)

**Verdict: all three fall short on the gates. None passes `just check`, and only OpenCode passes the merged PR's tests, but its turn never ended. Pi came closest: it finished, cost least, and failed one arch rule and one of the four merged-PR tests.**

## Setup

- Task: #361 ("hibernated ACP terminals from an older build read as protocol 0 and block every self-update"). Base c3e72af. Answer key: PR #363, merge 3dac368 (+86 −6 in 4 files). Each agent got #361's issue text as its only prompt.
- Model: `deepseek-flash` (DeepSeek-V4.1-Flash) for all three, through DeepSeek's OpenAI-format API.
- shed: staging 51cabf8 (includes #373), run as its own `shed serve --dir <scratch>`. Each agent ran as `shed run --acp <agent> --account trial --policy allow` on the account `deepseek/trial`, which came from `shed account add deepseek trial --token-file <0600 file>` (the file was deleted right after). Serve ran without the key in its environment, so the key reached the agents only through the account.
- Agents: jcode 1.0.3 from a local registry entry (`shed agents add jcode --registry <file>`), pi-acp 0.0.35 (with the `pi` CLI 1.6.0 on PATH), OpenCode 2.4.1. Each had a provider override in `<config>/providers/<id>.toml` (`agent = "deepseek"`, `token_var = "DEEPSEEK_API_KEY"`) plus its own model setting: `JCODE_PROVIDER`/`JCODE_MODEL`, Pi's `settings.json`, or `OPENCODE_CONFIG_CONTENT`.
- Spend: the balance was read through shed (`shed account status deepseek trial`) before each run, every minute during it, and two minutes after it. The stop rule was US$0.65 per agent, plus a 60-minute wall-time cap.
- Each run used its own clone with history only up to the base, on a `trial/<agent>` branch that was never pushed.

### Keeping the answer key out of reach

#361 is merged and public, and with `--policy allow` the agents looked for the fix. Three runs were voided, and their spend is still counted below:

| Void run | How it got the answer |
|---|---|
| jcode #1 | `git log --all` in a worktree that shared the repo's history, then `git show` of the merged commits |
| Pi #1 | It found this lane's own worktree on disk and ran `git show 3dac368` there |
| Pi #2 (file sandbox) | `git fetch https://github.com/jirathip-dev/shed main`, then it applied 3dac368 |

By the owner's decision (q3567), Pi #3 and OpenCode then ran under macOS `sandbox-exec`. The profile blocks reads of the other shed checkouts, the trial's scratch files and the live shed's state and socket, and allows outbound network only to localhost. A forwarder on localhost reaches only `api.deepseek.com`, and each agent's base URL pointed at it. Both agents tried GitHub (`curl https://api.github.com/...`) and were refused. jcode's scored run (#2) predates that sandbox: it used a base-only clone with no remote, and its transcript shows no lookup of the fix.

## Results

| Agent | `just check` | Merged PR's tests on the agent's branch (`cargo test -p shed --test acp restart::` with 3dac368's `restart.rs`) | Cost (balance delta) | Input / cached / output tokens | Cache hit | Wall time | Finished |
|---|---|---|---|---|---|---|---|
| jcode 1.0.3 (run #2) | fail: core/arch (3), local/generated, rust/clippy | fail: 2 of 4 pass | US$0.14 | 135,044 / 19,012,480 / 88,426 | 99.3 % | 785 s | finished |
| Pi (pi-acp 0.0.35, run #3) | fail: core/arch (1) | fail: 3 of 4 pass | US$0.08 | 190,463 / 9,449,344 / 55,621 | 98.0 % | 1207 s | finished |
| OpenCode 2.4.1 | fail: 7 gates, including a macOS test build that does not compile | pass: 4 of 4 | US$0.15 | 180,040 / 23,043,456 / 28,503 (+54,215 reasoning) | 99.2 % | 3622 s | unfinished (cancelled at the 60-minute cap) |

- **Tokens:** each agent's own session record (jcode's session JSON, Pi's session JSONL, OpenCode's message table). Input means uncached input, and the cache-hit rate is cached / (uncached + cached). DeepSeek reported no cache writes.
- **Cost:** the shed balance delta, which DeepSeek reports to the cent.
- **Wall time:** from `send` to the turn ending.
- **Control:** at 3dac368 itself, on the same host and build folder, `just check` passes all 32 gates and the merged-PR tests pass 4 of 4. Every failure below is the agent's own.
- **What each gate failed on:**
  - **jcode:** `registry/load.rs:load` (122 lines) and `lane.rs:run` (118 lines) are over the 105-line limit. `Instant::now` is used outside `clock.rs`. `CHANGELOG.md` was not regenerated from its fragment. Clippy flags `indexing_slicing` in `shed-server`. Merged-PR tests: `a_lane_that_speaks_an_older_protocol_still_blocks_a_handover` (the handover went through) and `lanes_saved_without_their_protocol_do_not_block_a_handover` (`send` to the hibernated lane after the handover: `INTERNAL`) fail.
  - **Pi:** `registry/load.rs:load` is 118 lines. Merged-PR test `lanes_saved_without_their_protocol_do_not_block_a_handover` fails the same way as jcode's.
  - **OpenCode:** `registry/load.rs:load` is 129 lines. `CHANGELOG.md` was not regenerated. The `local/test-*`, `local/idle-*` and `local/timing` gates stop at a `shed-server` build that cannot find `Terminal::adopt_as` and `is_child`.
- **OpenCode's ending:** its model sent its final message (finish `stop`) 1706 s after the prompt, but the ACP turn stayed open while a `opencode-pty` session it had started kept running. The run was cancelled at the cap.

## Claude reviewer, each diff against the merged PR

> jcode: partly. It fixes the root cause (no `speaks` reads as range 1..2, which today's floor of 1 covers) and tests all of it, but a recorded `speaks: 0` now passes too, plus a lot of extra scope.

> pi: partly. Its core fix matches the key (`unwrap_or(1)`), but it respawns any hibernated bridge that exits, killed ones included, and no test runs `handover.check` against a live terminal with no `speaks`.

> opencode: partly. It reads a missing `speaks` as the current protocol (5) instead of 1 and saves that into state.json, so raising the floor later would let real old bridges through. Its tests are the strongest of the three.

The reviewer also noted:
- All three add a restart-on-load for hibernated ACP terminals whose bridge is gone, which the merged fix does not.
- jcode and Pi change `serve --preflight` to report oldest 0.

## Spend

| Run | Balance before → after (US$) | Spent |
|---|---|---|
| jcode #1 (void) | 5.57 → 5.51 | 0.06 |
| jcode #2 | 5.51 → 5.37 | 0.14 |
| Pi #1 (void) | 5.37 → 5.34 | 0.03 |
| Pi #2 (void) | 5.33 → 5.30 | 0.03 |
| Pi #3 | 5.30 → 5.22 | 0.08 |
| OpenCode | 5.22 → 5.07 | 0.15 |

**Total spend: US$0.50** (5.57 → 5.07), under the US$2 cap. The extra US$0.01 between Pi #1 and Pi #2 is a balance reading that landed late.

## The account after the trial (#369, #372)

`shed account ls`:

```
ACCOUNT  STATE  WHO  USAGE  TOKEN  LINE
deepseek/trial  ready  -  5.07 USD  saved
```

`shed --json account ls`, the same row:

```json
{"provider": "deepseek", "name": "trial", "state": "ready", "balance": {"currency": "USD", "total": 5.07}, "spend": {"month": "2026-11", "tokens": {"input": 0, "cache_read": 0, "cache_write": 0, "output": 0}, "cost": {"amount": 0.145934118, "currency": "USD"}}}
```

This is the live run #369 and #372 lacked: all three agents ran on a key-only account (#372), and the balance was read through shed before and after every run. The month totals (#369) undercount, though: see the second follow-up.

## Observed problems and follow-ups (not fixed here)

- **The built-in `deepseek` record has no `id`.** `shed agents --json` lists it with an empty id, and a user override written without an `id` key loads under that same empty id instead of its file name.
- **Month spend misses most of the cost.** Tokens stay at 0 for all three agents: none of their records has a `[quota]` table, and ACP's `usage_update` has no token counts. The cost (US$0.146) is OpenCode's own reported cost only, while the balance fell US$0.50. A `[quota]` or `[price]` source for these agents, or the balance delta, would fix it.
- **Provider records for API-key lanes.** Running pi-acp, OpenCode or jcode on a `deepseek` account needs a user override (`agent = "deepseek"`, `token_var`, model settings). Shipping such records is a follow-up.
- **No built-in jcode entry yet.** jcode is not in the ACP registry, and its newest release with a macOS binary is 1.0.3 (1.1.x ships Linux only), so a built-in entry should wait.
- **Lanes run with host-wide access.** With `--policy allow`, agents read other checkouts, the live shed's `state.json` and sessions, sent a `hello` to the live socket, and ran a read-only `serve --preflight` against the live state folder (jcode #2). Nothing was changed. A sandbox option for ACP lanes (file and network) would stop this.
- **jcode:**
  - A `JCODE_HOME` path that goes through a symlink breaks its bash tool on every call (`could not load saved cloud environment`).
  - It then fell back to its "macos computer use" tool and ran shell commands on the host through AppleScript, writing into `/tmp` (jcode #1).
  - A long `JCODE_RUNTIME_DIR` fails with `path must be shorter than SUN_LEN`.
  - jcode#2116 (no reconnect after a daemon restart) and jcode#2088 (images) did not show up: no daemon restarted mid-run and no images were sent.
- **OpenCode:**
  - It ignores `HOME` and wrote to `~/.cache/opencode`, `~/.config/opencode`, `~/.local/share/opencode` and `~/.local/state/opencode` in the real home (no key stored there).
  - Its ACP turn did not end while a pty session stayed open.
- **Pi:** with `HOME` pointed at a scratch folder, rustup installed a second toolchain (1.4 GB) there. Set `CARGO_HOME` and `RUSTUP_HOME` explicitly when running an agent under a moved `HOME`.
