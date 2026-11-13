//! The drain bench's raw-read ceiling (`scripts/bench-drain.sh`): each stream `cat` into a PTY whose
//! output is read in 64 KiB chunks and dropped, timed until the PTY closes.
#![cfg(test)]

#[path = "../../shed/tests/drain/fixture.rs"]
mod fixture;

use std::io::Read as _;
use std::time::Instant;

#[test]
#[ignore = "a timing report: scripts/bench-drain.sh"]
fn drain_raw() {
    let dir = std::env::temp_dir().join(format!("shed-drain-raw-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    for kind in fixture::STREAMS {
        let file = fixture::write(&dir, kind);
        let (pty, pts) = pty_process::blocking::open().expect("pty");
        pty.resize(pty_process::Size::new(50, 200)).expect("size");
        let start = Instant::now();
        let mut child = pty_process::blocking::Command::new("cat")
            .arg(&file)
            .spawn(pts)
            .expect("cat");
        let mut buf = vec![0u8; 1 << 16];
        let mut bytes = 0u64;
        while let Ok(n @ 1..) = (&pty).read(&mut buf) {
            bytes += u64::try_from(n).expect("fits");
        }
        let secs = start.elapsed().as_secs_f64();
        child.wait().expect("cat exits");
        assert!(bytes >= std::fs::metadata(&file).expect("stream").len());
        fixture::report(&format!("raw {kind}"), bytes, secs);
    }
    std::fs::remove_dir_all(&dir).ok();
}
