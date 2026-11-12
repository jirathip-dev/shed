//! The drain bench's streams (`scripts/bench-drain.sh`): `$SHED_DRAIN_MB` (default 100) MB of plain
//! text, coloured text with `EL` on every line, and full-screen redraw frames, each ending in `drain-end`.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

pub const STREAMS: [&str; 3] = ["plain", "colour", "redraw"];
pub const END: &str = "drain-end";
const WORDS: [&str; 16] = [
    "alpha", "build", "cargo", "delta", "error", "frame", "green", "hello", "index", "jump",
    "kernel", "lines", "match", "north", "output", "pane",
];

pub fn megabytes() -> usize {
    std::env::var("SHED_DRAIN_MB").map_or(100, |v| v.parse().expect("SHED_DRAIN_MB is a number"))
}

struct Rng(u64);

impl Rng {
    fn below(&mut self, n: u64) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1);
        (self.0 >> 33) % n
    }

    fn words(&mut self, out: &mut String, n: u64) {
        for i in 0..n {
            if i > 0 {
                out.push(' ');
            }
            out.push_str(WORDS[usize::try_from(self.below(16)).expect("fits")]);
        }
    }
}

fn stream(kind: &str, bytes: usize) -> String {
    let mut rng = Rng(80);
    let mut out = String::with_capacity(bytes + 4096);
    let mut frame_row = 0;
    while out.len() < bytes {
        let colour = 31 + rng.below(7);
        match kind {
            "plain" => {
                let n = 4 + rng.below(11);
                rng.words(&mut out, n);
                out.push('\n');
            }
            "colour" => {
                write!(out, "\x1b[{colour}m").expect("write");
                let n = 4 + rng.below(11);
                rng.words(&mut out, n);
                out.push_str("\x1b[0m\x1b[K\n");
            }
            _ => {
                frame_row = frame_row % 50 + 1;
                write!(out, "\x1b[{frame_row};1H\x1b[{colour};1m").expect("write");
                let n = 8 + rng.below(20);
                rng.words(&mut out, n);
                out.push_str("\x1b[0m\x1b[K");
            }
        }
    }
    if kind == "redraw" {
        out.push_str("\x1b[H\x1b[2J");
    }
    out + END + "\n"
}

pub fn write(dir: &Path, kind: &str) -> PathBuf {
    let path = dir.join(format!("{kind}.txt"));
    std::fs::write(&path, stream(kind, megabytes() << 20)).expect("write the stream");
    path
}

pub fn report(what: &str, bytes: u64, secs: f64) {
    let mb = f64::from(u32::try_from(bytes).expect("fits")) / f64::from(1 << 20);
    println!("drain {what}: {:.0} ms, {:.0} MB/s", secs * 1e3, mb / secs);
}
