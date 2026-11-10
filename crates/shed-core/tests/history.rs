//! Bounded history under load.

use std::time::Instant;

use shed_core::history::{BYTES, LINES};
use shed_core::{History, Screen, Size, Terminal};

#[test]
fn throughput_report_with_full_scrollback() {
    let size = Size { rows: 24, cols: 80 };
    let mut screen = Screen::bounded(size, LINES, BYTES);
    let line = "é".repeat(39) + "\r\n";
    let chunk = line.repeat(8192 / line.len());
    for _ in 0..LINES {
        screen.feed(line.as_bytes());
    }
    let total = 16usize << 20;
    let start = Instant::now();
    let mut fed = 0;
    while fed < total {
        screen.feed(chunk.as_bytes());
        fed += chunk.len();
    }
    let secs = start.elapsed().as_secs_f64();
    let history = screen.history(usize::MAX);
    assert_eq!(history.lines().count(), LINES + size.rows as usize - 1);
    assert!(history.len() <= BYTES + 24 * 81);
    println!(
        "history throughput: {} MiB in {:.0} ms = {:.1} MiB/s (scrollback full: {LINES} lines)",
        fed >> 20,
        secs * 1e3,
        f64::from(u32::try_from(fed).expect("fits")) / secs / f64::from(1 << 20),
    );
}

#[test]
fn a_terminal_starts_from_its_log() {
    let dir = std::env::temp_dir().join(format!("shed-history-term-{}", std::process::id()));
    let log = dir.join("t.log");
    std::fs::create_dir_all(&dir).expect("dir");
    std::fs::write(&log, "before\r\n").expect("log");
    let history = History {
        log: Some(log.clone()),
        ..History::default()
    };
    let size = Size { rows: 5, cols: 20 };
    let t = Terminal::spawn_with(
        &["sh", "-c", "echo after; sleep 100"],
        &dir,
        [("A", "b")],
        size,
        &history,
    )
    .expect("spawn");
    let deadline = Instant::now() + std::time::Duration::from_secs(10);
    while !std::fs::read_to_string(&log)
        .expect("read")
        .ends_with("after\r\n")
    {
        assert!(Instant::now() < deadline, "the log never got the output");
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert_eq!(t.history(usize::MAX), "before\n\nafter");
    std::fs::remove_dir_all(dir).expect("clean");
}
