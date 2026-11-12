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

**Results.** Base 62e565a (bench from 97d4bc6), head 96f6921, release builds. Median of 5. Load average
7 to 12.

| 100 MiB stream | base | head | raw-read ceiling |
|---|---|---|---|
| plain | 2654 ms (38 MiB/s) | 932 ms (107 MiB/s) | 229–246 MiB/s |
| colour | 2618 ms (38 MiB/s) | 1009 ms (99 MiB/s) | 180–232 MiB/s |
| redraw | 1305 ms (77 MiB/s) | 1039 ms (96 MiB/s) | 144–234 MiB/s |
| serve peak RSS | 95–96 MB | 5–6 MB | |
| serve CPU, plain / colour / redraw | 2980 / 2840 / 1530 ms | 1690 / 1860 / 2010 ms | |

In an earlier run at load 4, the head drained plain and colour in 757 ms each (132 MiB/s), against
1911 ms and 1868 ms (52 and 54 MiB/s) at the base, with a raw ceiling of 202–248 MiB/s.

**Where the time goes.** In `sample` profiles of serve on the head, the PTY reader thread spends about
three quarters of its time inside `read(2)`. macOS hands out at most about 1 KiB per read, so 100 MiB
takes about 100,000 reads. The feeder thread (VT parse, screen, replies) is busy 80–85% of the time, and
the log writer about half. Each chunk handed to an idle thread costs a wake-up, which is why the
pipeline stays under the bare-PTY ceiling.

**What changed.** The base did everything on one thread: an 8 KiB read, the parse under the screen
lock, a log write. Scrolling built a `String` and a fresh 200-cell row for every line. Rotating the log
read its 8 MiB tail into memory. That buffering was the 90 MB RSS spike, which the allocator kept for
10–15 s. On the head:

- the PTY read, the screen and the log write run on three threads, linked by bounded queues;
- subscribers share one chunk;
- the scrollback is one ring of bytes;
- rows hold only the cells written to them, so a scroll rotates and clears a row;
- log rotation streams its tail from one file to the other.

Reads, `screen.watch` and attach output match the base on the golden files. Fed 20 MiB of each
stream, with two resizes along the way, the screen gives the base's relayed bytes, text, history and
snapshot.
