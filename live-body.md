Closes #80

## For humans

When a program prints a lot of output, shed now gets through it about 3.4 times faster on the bench host. The background server's memory no longer jumps by about 90 MB while it does.
A new benchmark measures this on every pull request and fails one that makes it more than 10% slower. Its numbers are in docs/bench.md.

## What changed

- **Bench** (commit 1, `baseline:`, no product code): `crates/shed/tests/drain_bench.rs` cats 100 MiB of plain, coloured (SGR + `EL`) and full-screen-redraw output into a fresh `shed serve`'s terminal. It times until `read` shows the last line and records serve's peak RSS and CPU time. `crates/shed-core/tests/drain_raw.rs` gives the raw-read ceiling. `scripts/bench-drain.sh base head` runs 5 interleaved release runs per side and fails on a 10% regression. CI runs it in a new `bench-drain` job.
- **terminal**: the PTY read, the screen feed (parse, subscribers, replies) and the log write run on three threads. They are joined by bounded queues of 256 chunks, with one screen lock and one watch bump per batch. Subscribers share one `Arc<[u8]>` per chunk, and nothing is copied when no one is attached.
- **history**: log rotation streams the kept tail from one file to the other. It used to read 8 MiB into memory on each rotation, and that buffering was the RSS spike.
- **screen**: scrollback is one `VecDeque<u8>` plus line lengths, written through a reused line buffer. Rows keep only the cells written to them, and the rest read as blanks, so a scroll rotates and clears a row instead of allocating 200 cells. The answered-query cut scans for ESC only when a query was answered.
- **docs/bench.md**: the table, the method, the machine and the commits.

`src/ net +93` (budget: none given in the issue; cap total 7761 / 7800). Most of it is the three-thread pipeline and the short-row handling.

## Before / after

Command: `BENCH_TARGET=<scratch>/bench scripts/bench-drain.sh <checkout of 62e565a> .` (bench from the `baseline:` commit 97d4bc6). Head 3563620 (src same as 9cb8147), Apple M6, 12 cores, load average 7 to 9. Median of 5, script exit 0:

| 100 MiB | base | head | change |
|---|---|---|---|
| plain | 2462 ms (41 MiB/s) | 722 ms (139 MiB/s) | 3.4x faster |
| colour | 2674 ms (37 MiB/s) | 826 ms (121 MiB/s) | 3.2x faster |
| redraw | 1255 ms (80 MiB/s) | 749 ms (134 MiB/s) | 1.7x faster |
| serve peak RSS | 95–96 MB | 10–13 MB | |
| raw-read ceiling | 268–272 / 246–252 / 247–257 MiB/s | | |

At load 4 (same queues, before the log-writer merge) plain and colour drained in 694 / 684 ms (148 / 150 MiB/s). Round 1's head 7989ce6 measured 932 ms plain at load 7–12.

## Fix round 2 (finding: AC3 not met)

- **terminal:** both queues hold 256 chunks instead of 16 (16 KiB of slack at macOS's ~1 KiB reads). The log writer merges queued chunks into one write. A profile showed the feeder blocked ~10% of the time on a full log queue during log rotation.
- **screen:** a print at the end of a row pushes the cell; scrolling copies a plain glyph without walking its four slots. Feeder instructions per 100 MiB: plain 13.6 → 10.7 G, colour 12.8 → 10.4 G.
- **AC3 pushback:** AC3 names "the machine of the table above", the issue's M4, which this lane cannot reach. On this shared M6, plain is 139 MiB/s median at load 7–9, with single runs up to 172. That is close to 150 but not reliably over it. The base runs 2.3–2.5x slower here than the issue's 1.05 s, and the head is 3.4x the base in the same interleaved runs. The VT parse is a tenth of the feeder's work, so the `alacritty_terminal` step would not close the gap. Owner call: accept, or run the bench on the M4.

## AC3: owner decision q1567

> Owner runs scripts/bench-drain.sh on the M4 and posts the numbers; merge if plain and colour reach 150

Command for the M4: `BENCH_TARGET=<scratch>/bench scripts/bench-drain.sh <checkout of 62e565a> .` at f519417. AC3 holds if the head's `drain plain` and `drain colour` medians are 150 MB/s or more.

## Evidence at f519417 (merged origin/staging 38c6449)

- `just fmt clippy banned cap arch comments`: exit 0 each. At 0531eb7 (same src as 62c6e70), `just doc deps hygiene neutral self-hosted lessons`: exit 0 each.
- `cargo test --workspace --locked --no-fail-fast`: exit 101, with two failures:
  - `machine_stats_here_and_on_a_machine_and_an_offline_one_answers_within_5_s` (`uptime_s` 0 on this host, as at base);
  - `serve_raises_a_lowered_soft_limit_so_many_sessions_spawn` (new from staging), `pty: Device not configured` at session 283. The host ran out of PTYs while other lanes held theirs. Run alone, the test passes at the head (`cargo test -p shed --test reliability serve_raises`, exit 0 twice) and on staging 38c6449 (exit 0).
- `just temp-left`: exit 1, but every folder it found is another lane's `shed-watch-*` from Nov 10–11.
- `cargo deny` and `actionlint` are not installed on this host; CI runs both.

## Why (round 2)

Decided: reader-to-feeder and feeder-to-log queues hold 256 chunks, not 16; 16 KiB of slack let a log rotation stall back up into cat
Because: crates/shed-core/src/terminal.rs:358
Decided: the log writer merges all queued chunks into one write, like the feeder
Because: crates/shed-core/src/terminal.rs:367
Decided: a print at the end of a row pushes the cell; history copies a plain glyph directly
Because: crates/shed-core/src/screen.rs:529
Rejected: alacritty_terminal behind a trait — the VT parse is 1-2 G of ~10 G instructions per 100 MiB; the rest is shed's grid
Rejected: a higher PTY line speed — macOS still hands out ~1 KiB per read at 38400 and 230400
Rejected: an ASCII fast path at the top of print — 7% fewer instructions on plain, 10% more on redraw
Check: AC3 on this shared M6 (load 7-9): plain 722 ms (139 MiB/s), colour 826 ms (121 MiB/s), single runs up to 172; load 4 gave 148/150 MiB/s
Check: AC3 names the issue's M4; here the base runs 2.3-2.5x slower than its 1.05 s, and the head is 3.4x faster than base in the same runs
Check: peak RSS 10-13 MB during a 100 MiB burst (limit 32 MB)
Owner: big output bursts now drain about 3.4 times faster than before this work, close to but not reliably above 150 MB/s on the busy build host

<sub>note on 9cb8147 · git log --notes=why</sub>

## Why (round 4)

Decided: AC3 is settled by the owner's M4 run of scripts/bench-drain.sh, merge if plain and colour reach 150
Because: q1567
Decided: merged origin/staging 38c6449 so the M4 run measures the current head
Because: crates/shed-core/src/terminal.rs:149
Decided: docs/bench.md claims output parity only through the existing golden tests
Because: docs/bench.md:61
Check: serve_raises_a_lowered_soft_limit_so_many_sessions_spawn passes alone at head and staging; it failed once in the full run when the host ran out of PTYs
Check: machine_stats fails on this host only (uptime_s 0), as at base
Rejected: more tuning on the shared M6 — the owner chose an M4 run over more optimisation (q1567)
Owner: run the drain bench on the M4 at f519417; merge if plain and colour reach 150 MB/s

<sub>note on f519417 · git log --notes=why</sub>

