use std::io::{BufRead as _, BufReader};
use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

use serde_json::json;
use shed_client::MIN_PROTOCOL;
use shed_server::proto::{BUILD, LONG_VERSION, PROTOCOL};

use crate::common::{Server, temp_dir};
use crate::{shed, shed_in, stdout};

#[test]
fn hello_prints_the_version() {
    let dir = temp_dir("hello");
    let out = shed_in(&dir, &["hello"]);
    assert_eq!(stdout(&out), format!("shed {LONG_VERSION}\n"));
    let _server = Server::start(&dir);
    assert_eq!(
        stdout(&shed_in(&dir, &["hello"])),
        format!("shed {LONG_VERSION}\n")
    );
}

#[test]
fn hello_reports_a_server_on_another_build() {
    let dir = temp_dir("hello-build");
    let listener = std::os::unix::net::UnixListener::bind(dir.join("shed.sock")).expect("bind");
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept");
        let mut hello = String::new();
        BufReader::new(&stream)
            .read_line(&mut hello)
            .expect("hello");
        let other = json!({"jsonrpc": "2.0", "id": 1, "result":
            {"product": "shed", "version": "0.0.0", "build": "56de739", "protocol": PROTOCOL}});
        std::io::Write::write_all(&mut stream, format!("{other}\n").as_bytes()).expect("reply");
    });
    let out = shed_in(&dir, &["hello"]);
    server.join().expect("fake server");
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        format!("shed {LONG_VERSION}\n")
    );
    assert_eq!(
        String::from_utf8_lossy(&out.stderr),
        format!("shed serve runs build 56de739, this is {BUILD}; restart shed serve\n")
    );
}

#[test]
fn schema_prints_the_committed_openrpc_document() {
    let committed = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/shed.openrpc.json");
    let committed = std::fs::read_to_string(committed).expect("docs/shed.openrpc.json");
    assert_eq!(shed(&["schema"]), committed);
}

#[test]
fn version_flag_prints_the_version_and_the_git_sha() {
    let sha = git_sha();
    assert_eq!(
        shed(&["--version"]),
        format!("shed {} ({sha})\n", env!("CARGO_PKG_VERSION"))
    );
}

fn git_sha() -> String {
    let out = Command::new("git")
        .args(["rev-parse", "--short=7", "HEAD"])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("git runs");
    String::from_utf8(out.stdout)
        .expect("utf-8")
        .trim()
        .to_owned()
}

#[test]
fn a_client_refuses_a_server_below_its_oldest_protocol_at_once_naming_both_ranges() {
    let dir = temp_dir("protocol");
    let listener = std::os::unix::net::UnixListener::bind(dir.join("shed.sock")).expect("bind");
    let (done, held) = std::sync::mpsc::channel::<()>();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept");
        let mut hello = String::new();
        BufReader::new(&stream)
            .read_line(&mut hello)
            .expect("hello");
        let older = json!({"jsonrpc": "2.0", "id": 1, "result":
            {"product": "shed", "version": "0.0.0", "build": "56de739", "protocol": PROTOCOL - 2}});
        std::io::Write::write_all(&mut stream, format!("{older}\n").as_bytes()).expect("reply");
        held.recv_timeout(Duration::from_secs(10)).ok();
    });
    let started = Instant::now();
    let out = shed_in(&dir, &["ls"]);
    let took = started.elapsed();
    done.send(()).ok();
    server.join().expect("fake server");
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    assert_eq!(
        String::from_utf8_lossy(&out.stderr),
        format!(
            "shed: SHED_VERSION: this client speaks shed protocol {MIN_PROTOCOL} to {PROTOCOL}; the server is 0.0.0 (56de739) protocol {0} to {0}; update shed there (shed updates itself from its own releases: `shed self-update`, docs/self-update.md)\n",
            PROTOCOL - 2
        )
    );
    assert!(took < Duration::from_secs(2), "took {took:?}");
}

#[test]
fn help_lists_every_command() {
    let help = shed(&["--help"]);
    for cmd in [
        "hello", "schema", "serve", "run", "ls", "get", "read", "send", "keys", "kill", "rm",
        "attach", "state", "wait", "events", "top", "approve", "cancel", "agents", "connect",
        "machine",
    ] {
        assert!(
            help.contains(&format!("\n  {cmd} ")),
            "{cmd} missing from {help}"
        );
    }
}
