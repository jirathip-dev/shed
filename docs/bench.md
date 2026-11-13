# Benches

## Drain: heavy output into one terminal (#80)

How fast serve drains a burst of output, and how much memory it takes while it does.

**Method.** `scripts/bench-drain.sh <base> <head>` builds both checkouts in release mode and runs
`crates/shed/tests/drain_bench.rs` 5 times on each, interleaved. Each run starts a fresh `shed serve`
for each stream, `cat`s 100 MiB into a 50x200 terminal (`sh -c 'cat FILE; exec sleep 600'`) and times
until `read` (polled every 2 ms) shows the stream's last line. While it drains, serve's RSS is sampled
every 10 ms with `ps` for the peak, and its CPU time is read before and after. The streams
(`crates/shed/tests/drain/fixture.rs`) are:

- **plain**: lines of 4 to 14 words;
- **colour**: the same with an SGR colour, a reset and `EL` on every line;
- **redraw**: full-screen frames, each row positioned with `CUP` and coloured, ended with `EL`.

The raw-read ceiling (`crates/shed-core/tests/drain_raw.rs`) is the same `cat` into a bare PTY, read in
64 KiB reads and dropped. CI runs the script on every PR (`bench-drain`) and fails if the head is more
than 10% slower than the base on any stream. Set `SHED_DRAIN_MB` for another size.

**Machine.** Apple M6 (Mac18,5), 12 cores, 32 GB, macOS 27.2. This is a shared build host, so its load
average during the runs is given too. The numbers move with that load by up to 2x.

**Results.** Base 62e565a (bench from 97d4bc6), head 3563620, release builds. Median of 5. Load average
7 to 9.

| 100 MiB stream | base | head | raw-read ceiling |
|---|---|---|---|
| plain | 2462 ms (41 MiB/s) | 722 ms (139 MiB/s) | 268–272 MiB/s |
| colour | 2674 ms (37 MiB/s) | 826 ms (121 MiB/s) | 246–252 MiB/s |
| redraw | 1255 ms (80 MiB/s) | 749 ms (134 MiB/s) | 247–257 MiB/s |
| serve peak RSS | 95–96 MB | 10–13 MB | |
| serve CPU, plain / colour / redraw | 2720 / 2820 / 1600 ms | 1200 / 1440 / 1410 ms | |

Single head runs ranged from 120 to 172 MiB/s for plain. An earlier run at load 4, with the same
queues but before the log writer merged its chunks, drained plain in 694 ms and colour in 684 ms
(148 and 150 MiB/s) against 2013 and 1830 ms at the base. The previous head, 7989ce6 with 16-chunk
queues, drained plain at 107 MiB/s at load 7 to 12.

**Where the time goes.** In `sample` profiles of serve, the PTY reader thread spends about 90% of its
time inside `read(2)`. macOS hands out about 1 KiB per read, at any line speed, so 100 MiB takes about
100,000 reads. The feeder thread (VT parse, screen, replies) is busy about 80% of the time. Of its
work, the VT parse is a tenth (1 to 2 G of about 10 G instructions per 100 MiB). The rest is the
screen grid and the scrollback, so another VT parser would not close the gap. The log writer is about
half busy, and rotating the log stalls it for a moment. With 16-chunk queues (16 KiB) such a stall
blocked the feeder, then the reader, then `cat`; 256-chunk queues absorb it.

**What changed.** The base did everything on one thread: an 8 KiB read, the parse under the screen
lock, a log write. Scrolling built a `String` and a fresh 200-cell row for every line. Rotating the log
read its 8 MiB tail into memory. That buffering was the 90 MB RSS spike, which the allocator kept for
10–15 s. On the head:

- the PTY read, the screen and the log write run on three threads, linked by queues of 256 chunks,
  and the feeder and the log writer each take all queued chunks at once;
- subscribers share one chunk;
- the scrollback is one ring of bytes;
- rows hold only the cells written to them, so a scroll rotates and clears a row;
- log rotation streams its tail from one file to the other.

Reads, `screen.watch` and attach output match the base on the golden files. Fed 20 MiB of each
stream, with two resizes along the way, the screen gives the base's relayed bytes, text, history and
snapshot.
