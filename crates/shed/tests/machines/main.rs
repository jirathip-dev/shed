//! Machines over SSH end to end: `shed connect` over a socketpair (simulated SSH), and `--on` through a
//! fake `ssh` that runs `shed connect` against a second server in a temp dir.
//!
//! The CLI output is checked against `tests/golden/machines.txt` (the temp dir and version
//! normalised). Regenerate with `just generate` (the tests in `golden.rs`).
#![cfg(test)]

mod acp;
#[path = "../common/mod.rs"]
mod common;
mod connect;
mod exec;
mod golden;
mod link;
mod proxy;
mod script;
mod split;
mod stats;

use std::io::BufRead as _;
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use common::Server;
use serde_json::Value;
use shed_server::proto::{EventsParams, Hello, Method, PROTOCOL, Response, RpcError};

const SHED: &str = env!("CARGO_BIN_EXE_shed");

const VERSION: &str = env!("CARGO_PKG_VERSION");

const OLD: &str = "56de739";

const FORWARD: &str = r#"import os, socket, sys, threading
remote, local = sys.argv[1:3]
log = os.path.join(os.path.dirname(os.path.abspath(__file__)), "sent.bytes")
server = socket.socket(socket.AF_UNIX)
server.bind(remote)
server.listen()
def pipe(a, b, kept):
    while data := a.recv(65536):
        if kept:
            open(log, "ab").write(data)
        b.sendall(data)
    try:
        b.shutdown(socket.SHUT_WR)
    except OSError:
        pass
def serve():
    while True:
        conn, _ = server.accept()
        up = socket.socket(socket.AF_UNIX)
        up.connect(local)
        threading.Thread(target=pipe, args=(conn, up, False), daemon=True).start()
        threading.Thread(target=pipe, args=(up, conn, True), daemon=True).start()
threading.Thread(target=serve, daemon=True).start()
sys.stdin.buffer.read()
"#;

fn script(path: &Path, body: &str) {
    std::fs::create_dir_all(path.parent().expect("parent")).expect("dir");
    std::fs::write(path, format!("#!/bin/sh\n{body}")).expect("write script");
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).expect("chmod");
}

fn fake_hosts(root: &Path) -> PathBuf {
    let bin = root.join("bin");
    script(
        &bin.join("ssh"),
        "# A fake ssh: a forward (-R) relays the remote socket to the local one, here, until stdin ends;\n\
         # else it logs its arguments, skips the options, runs the command on the target's PATH.\n\
         case \" $* \" in *\" -R \"*) while [ \"$1\" != -R ]; do shift; done\n\
         exec python3 \"$(dirname \"$0\")/forward.py\" \"${2%%:*}\" \"${2#*:}\";; esac\n\
         echo \"$*\" >> \"$(dirname \"$0\")/ssh.log\"\n\
         echo $$ >> \"$(dirname \"$0\")/ssh.pids\"\n\
         while [ \"$1\" = -o ]; do shift 2; done\n\
         [ \"$1\" = -- ] && shift\n\
         target=$1\nshift\n\
         PATH=\"$(dirname \"$0\")/../$target:$PATH\" exec \"$@\"\n",
    );
    std::fs::write(bin.join("forward.py"), FORWARD).expect("write forward.py");
    let remote = root.join("remote");
    script(
        &root.join("user@box.example/shed"),
        &format!("exec '{SHED}' --dir '{}' \"$@\"\n", remote.display()),
    );
    let old = Hello {
        build: OLD.into(),
        protocol: PROTOCOL - 1,
        ..Hello::ours()
    };
    let refusal = Response::err(1.into(), RpcError::version(&Hello::ours(), &old));
    let refusal = serde_json::to_string(&refusal).expect("json");
    script(
        &root.join("old.example/shed"),
        &format!("read -r line\nprintf '%s\\n' '{refusal}'\n"),
    );
    bin
}

fn shed_on(root: &Path, args: &[&str]) -> Command {
    let path = std::env::var("PATH").unwrap_or_default();
    let mut cmd = Command::new(SHED);
    cmd.arg("--dir")
        .arg(root.join("local"))
        .args(args)
        .env("PATH", format!("{}:{path}", root.join("bin").display()));
    cmd
}

fn linked(root: &Path) -> (Server, Server) {
    fake_hosts(root);
    let remote = Server::start(&root.join("remote"));
    for (name, target) in [("box", "user@box.example"), ("old", "old.example")] {
        let added = shed_on(root, &["machine", "add", name, target]).output();
        assert!(added.expect("machine add").status.success());
    }
    let path = format!(
        "{}:{}",
        root.join("bin").display(),
        std::env::var("PATH").unwrap()
    );
    let local = Server::with_env(&root.join("local"), "3600", &[("PATH", &path)]);
    common::eventually("the link to box", || link(root, "box")["state"] == "up");
    (remote, local)
}

fn link(root: &Path, name: &str) -> Value {
    let out = shed_on(root, &["--json", "machine", "ls"])
        .output()
        .expect("ls");
    let list: Value = serde_json::from_slice(&out.stdout).expect("JSON");
    let list = list.as_array().expect("a list").iter();
    list.into_iter()
        .find(|m| m["name"] == name)
        .map_or(Value::Null, |m| m["link"].clone())
}

fn ok(root: &Path, args: &[&str]) -> String {
    let out = shed_on(root, args).output().expect("shed runs");
    assert!(out.status.success(), "shed {args:?}: {out:?}");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn box_ssh(root: &Path) -> usize {
    let log = std::fs::read_to_string(root.join("bin/ssh.log")).unwrap_or_default();
    log.lines()
        .filter(|l| l.contains("user@box.example"))
        .count()
}

fn local_events(root: &Path, since: u64) -> impl Iterator<Item = Value> {
    let socket = shed_server::Paths::in_dir(&root.join("local")).socket;
    let mut client = shed_client::Conn::connect(&socket).expect("connect");
    let params = EventsParams {
        since: Some(since),
        ..EventsParams::default()
    };
    let _: Value = client.call(Method::Events, &params).expect("events");
    let (reader, _writer) = client.split();
    let timeout = Some(Duration::from_secs(20));
    reader.get_ref().set_read_timeout(timeout).expect("timeout");
    reader
        .lines()
        .map(|l| serde_json::from_str(&l.expect("an event line")).expect("event JSON"))
}
