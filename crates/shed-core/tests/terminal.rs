//! Terminals on real PTYs.
#![cfg(test)]

use std::collections::BTreeSet;
use std::path::Path;
use std::time::{Duration, Instant};

use shed_core::proctree::{self, ProcessTable as _};
use shed_core::{Size, Terminal};

const SIZE: Size = Size { rows: 24, cols: 80 };
const NO_ENV: [(&str, &str); 0] = [];

fn sh(script: &str, size: Size) -> Terminal {
    Terminal::spawn(&["sh", "-c", script], Path::new("/"), NO_ENV, size).expect("spawn")
}

fn eventually(what: &str, mut check: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !check() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn alive(pid: i32) -> bool {
    let procs = proctree::system().list().expect("process list");
    procs.iter().any(|p| p.pid == pid && !p.zombie)
}

#[test]
fn reads_printf_output_exactly() {
    let t = sh(
        "printf 'hello\\nworld\\n'; printf 'a\\tb\\033[2D\\033[Kc'; sleep 100",
        SIZE,
    );
    eventually("output", || t.screen().ends_with('c'));
    assert_eq!(t.screen(), "hello\nworld\na      c");
}

#[test]
fn passes_cwd_env_and_input() {
    let t =
        Terminal::spawn(&["sh"], Path::new("/tmp"), [("SHED_X", "x-value")], SIZE).expect("spawn");
    t.write(b"echo \"$SHED_X $(basename \"$PWD\") $TERM\"")
        .expect("write");
    t.press("enter").expect("enter");
    eventually("echo", || t.screen().contains("x-value tmp xterm-256color"));
    t.press("ctrl+d").expect("ctrl+d");
    eventually("exit", || t.try_wait().expect("try_wait").is_some());
}

#[test]
fn resize_reaches_the_program_and_the_screen() {
    let t = sh(
        "trap 'stty size' WINCH; stty size; while :; do sleep 0.05; done",
        Size { rows: 10, cols: 40 },
    );
    eventually("first size", || t.screen() == "10 40");
    t.resize(Size { rows: 5, cols: 20 }).expect("resize");
    eventually("new size", || t.screen().ends_with("5 20"));
}

#[test]
fn scrollback_is_bounded() {
    let t = sh(
        "i=0; while [ $i -lt 5050 ]; do echo line$i; i=$((i+1)); done; sleep 100",
        Size { rows: 5, cols: 20 },
    );
    eventually("all lines", || t.screen().ends_with("line5049"));
    let history = t.history(usize::MAX);
    let lines: Vec<&str> = history.lines().collect();
    assert_eq!(lines.len(), shed_core::history::LINES + 4);
    assert_eq!(lines.first(), Some(&"line46"));
    assert_eq!(t.history(2), "line5048\nline5049");
}

#[test]
fn kill_ends_the_whole_tree() {
    let t = sh("sleep 100 & wait", SIZE);
    let mut grandchild = None;
    eventually("grandchild", || {
        let procs = proctree::system().list().expect("process list");
        grandchild = proctree::tree(&procs, t.pid(), &BTreeSet::new())
            .iter()
            .map(|p| p.pid)
            .find(|&p| p != t.pid());
        grandchild.is_some()
    });
    let grandchild = grandchild.expect("grandchild pid");
    let usage = t.usage().expect("usage");
    assert!(usage.processes >= 2 && usage.rss_bytes > 0, "{usage:?}");

    t.kill(Duration::from_secs(2)).expect("kill");
    assert!(!alive(t.pid()));
    eventually("grandchild gone", || !alive(grandchild));
}

#[test]
fn kill_escalates_to_sigkill_after_the_grace_period() {
    let t = sh(
        "trap '' TERM; echo ready; while :; do sleep 0.05; done",
        SIZE,
    );
    eventually("ready", || t.screen() == "ready");
    let start = Instant::now();
    let status = t.kill(Duration::from_millis(300)).expect("kill");
    assert!(start.elapsed() >= Duration::from_millis(300));
    assert!(!status.success());
    assert!(!alive(t.pid()));
}

struct OutOfFiles;

impl proctree::ProcessTable for OutOfFiles {
    fn list(&self) -> std::io::Result<Vec<proctree::ProcessInfo>> {
        Err(std::io::Error::from_raw_os_error(24))
    }
}

#[test]
fn kill_ends_the_group_when_the_process_table_cannot_be_read() {
    let t = sh("echo ready; sleep 100", SIZE);
    eventually("ready", || t.screen() == "ready");
    drop(proctree::kill_tree(
        &OutOfFiles,
        t.pid(),
        Duration::from_millis(300),
        || {},
    ));
    eventually("the program exited", || {
        t.try_wait().expect("try_wait").is_some()
    });
}

#[test]
fn subscribers_get_raw_output_until_the_pty_closes() {
    let t = sh("read line; printf \"got:$line\\033[1m\\n\"", SIZE);
    let mut rx = t.subscribe();
    t.write(b"x\n").expect("write");
    let mut out = Vec::new();
    while let Some(chunk) = rx.blocking_recv() {
        out.extend_from_slice(&chunk);
    }
    let out = String::from_utf8(out).expect("utf-8");
    assert!(
        out.contains("got:x\u{1b}[1m"),
        "raw bytes, escapes kept: {out:?}"
    );
    assert!(
        t.subscribe().blocking_recv().is_none(),
        "closed once the PTY has"
    );
}
