use std::cell::RefCell;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use shed_server::proto::{LsParams, Method, TerminalInfo};
use shed_server::{Client, Paths};

struct Made(RefCell<Vec<PathBuf>>);

impl Drop for Made {
    fn drop(&mut self) {
        for dir in self.0.borrow().iter() {
            std::fs::remove_dir_all(dir).ok();
        }
    }
}

thread_local!(static MADE: Made = const { Made(RefCell::new(Vec::new())) });

pub fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("shed-{name}-{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    std::fs::create_dir_all(&dir).expect("temp dir");
    MADE.with(|m| m.0.borrow_mut().push(dir.clone()));
    dir.canonicalize().expect("canonical temp dir")
}

pub fn eventually(what: &str, mut check: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !check() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

pub struct Server(pub Child, PathBuf);

impl Server {
    pub fn start(dir: &Path) -> Self {
        Self::sampling(dir, "3600")
    }

    pub fn sampling(dir: &Path, secs: &str) -> Self {
        Self::with_env(dir, secs, &[])
    }

    pub fn with_env(dir: &Path, secs: &str, env: &[(&str, &str)]) -> Self {
        let child = Command::new(env!("CARGO_BIN_EXE_shed"))
            .args(["serve", "--sample", secs, "--dir"])
            .arg(dir)
            .env("SHELL", "/bin/sh")
            .env("PS1", "$ ")
            .env_remove("ENV")
            .envs(env.iter().copied())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("shed serve starts");
        let socket = Paths::in_dir(dir).socket;
        let server = Self(child, socket.clone());
        eventually("the socket", || UnixStream::connect(&socket).is_ok());
        server
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let lanes: Vec<TerminalInfo> = Client::connect(&self.1)
            .and_then(|mut c| c.call(Method::Ls, &LsParams::default()))
            .unwrap_or_default();
        for pid in lanes
            .iter()
            .filter(|t| t.acp.is_some())
            .filter_map(|t| t.pid)
        {
            let group = format!("-{pid}");
            Command::new("kill")
                .args(["-KILL", "--", &group])
                .status()
                .ok();
        }
        self.0.kill().ok();
        self.0.wait().ok();
    }
}
