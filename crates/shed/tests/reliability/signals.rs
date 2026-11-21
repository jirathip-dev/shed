use std::os::unix::net::UnixStream;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use shed_server::Paths;

use crate::common::{eventually, temp_dir};
use crate::{SHED, alive, ok, pids, run_sh, run_shell};

struct Serve(Child);

impl Serve {
    fn start(dir: &Path, prelude: &str, stderr: Stdio) -> Self {
        let script = format!("{prelude} exec \"$0\" serve --sample 3600 --dir \"$1\"");
        let child = Command::new("sh")
            .args(["-c", &script, SHED])
            .arg(dir)
            .env("SHELL", "/bin/sh")
            .stdout(Stdio::null())
            .stderr(stderr)
            .spawn()
            .expect("shed serve");
        let server = Self(child);
        let socket = Paths::in_dir(dir).socket;
        eventually("the socket", || UnixStream::connect(&socket).is_ok());
        server
    }

    fn signal(&self, sig: &str) {
        let pid = self.0.id().to_string();
        let sent = Command::new("kill").args([sig, &pid]).status();
        assert!(sent.expect("kill").success());
    }
}

impl Drop for Serve {
    fn drop(&mut self) {
        self.0.kill().ok();
        self.0.wait().ok();
    }
}

fn survivors(trees: &[(String, String)], within: Duration) -> Vec<String> {
    let deadline = Instant::now() + within;
    loop {
        let live: Vec<String> = trees
            .iter()
            .flat_map(|(pid, child)| [pid, child])
            .filter(|p| alive(p))
            .cloned()
            .collect();
        if live.is_empty() || Instant::now() > deadline {
            for pid in &live {
                Command::new("kill").args(["-KILL", pid]).status().ok();
            }
            return live;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn sigterm_to_serve_ends_every_tree_even_one_that_ignores_sighup() {
    let dir = temp_dir("rel-sigterm");
    let mut server = Serve::start(&dir, "", Stdio::null());
    run_sh(&dir, "a", "sleep 100 & echo child=$!; wait");
    run_sh(&dir, "b", "sleep 100 & echo child=$!; wait");
    run_sh(&dir, "deaf", "trap '' HUP; sleep 100 & echo child=$!; wait");
    let trees = ["a", "b", "deaf"].map(|n| pids(&dir, n));
    server.signal("-TERM");
    let left = survivors(&trees, Duration::from_secs(3));
    assert!(left.is_empty(), "alive 3 s after SIGTERM: {left:?}");
    server.0.wait().expect("serve exits");
}

#[test]
fn a_program_gets_default_signals_from_a_serve_that_ignores_sighup() {
    let dir = temp_dir("rel-sigdfl");
    let _server = Serve::start(&dir, "trap '' HUP USR1;", Stdio::null());
    let d = dir.to_str().expect("utf-8 dir");
    let script = "import signal as s, time\n\
        print('hup', s.getsignal(s.SIGHUP).name, 'usr1', s.getsignal(s.SIGUSR1).name, \
        'blocked', len(s.pthread_sigmask(s.SIG_BLOCK, [])), flush=True)\n\
        time.sleep(60)";
    ok(
        &dir,
        &[
            "run", "--name", "p", "--cwd", d, "--", "python3", "-c", script,
        ],
    );
    let mut line = String::new();
    eventually("the program's dispositions", || {
        let screen = ok(&dir, &["read", "p"]);
        let found = screen.lines().find(|l| l.starts_with("hup "));
        found.unwrap_or_default().clone_into(&mut line);
        !line.is_empty()
    });
    assert_eq!(line.trim_end(), "hup SIG_DFL usr1 SIG_DFL blocked 0");
}

#[test]
fn the_next_start_ends_a_tree_a_killed_serve_left_behind() {
    let dir = temp_dir("rel-left");
    let first = Serve::start(&dir, "", Stdio::null());
    run_sh(&dir, "deaf", "trap '' HUP; sleep 100 & echo child=$!; wait");
    let tree = pids(&dir, "deaf");
    drop(first);
    assert!(alive(&tree.0) && alive(&tree.1), "left behind: {tree:?}");
    let log = dir.join("serve.log");
    let err = std::fs::File::create(&log).expect("log file");
    let _second = Serve::start(&dir, "", Stdio::from(err));
    let left = survivors(std::slice::from_ref(&tree), Duration::from_secs(3));
    assert!(left.is_empty(), "alive after the next start: {left:?}");
    let text = std::fs::read_to_string(&log).expect("log");
    let named: Vec<&str> = text.lines().filter(|l| l.contains("deaf")).collect();
    assert_eq!(named.len(), 1, "{text}");
    assert!(named.iter().all(|l| l.contains(&tree.0)), "{text}");
}

#[test]
fn sigterm_to_serve_with_20_shells_ends_in_under_a_second() {
    let dir = temp_dir("rel-shells");
    let mut server = Serve::start(&dir, "", Stdio::null());
    for i in 0..20 {
        run_shell(&dir, &format!("s{i}"));
    }
    let started = Instant::now();
    server.signal("-TERM");
    server.0.wait().expect("serve exits");
    let took = started.elapsed();
    assert!(took < Duration::from_secs(1), "serve took {took:?}");
}
