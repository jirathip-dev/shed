use std::io::{BufRead as _, BufReader, Write as _};
use std::os::fd::OwnedFd;
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::process::{Command, Stdio};

use shed_server::proto::BUILD;

use crate::common::{Server, temp_dir};
use crate::{SHED, VERSION, script, shed_on};

#[test]
fn connect_carries_the_protocol_over_a_socketpair() {
    let dir = temp_dir("connect");
    let _server = Server::start(&dir);
    let (ours, theirs) = UnixStream::pair().expect("socketpair");
    let mut child = Command::new(SHED)
        .arg("--dir")
        .arg(&dir)
        .arg("connect")
        .stdin(Stdio::from(OwnedFd::from(
            theirs.try_clone().expect("clone"),
        )))
        .stdout(Stdio::from(OwnedFd::from(theirs)))
        .spawn()
        .expect("shed connect starts");
    let mut reader = BufReader::new(ours.try_clone().expect("clone"));
    let golden = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden/hello.jsonl");
    let golden = std::fs::read_to_string(golden).expect("hello golden");
    let exchange: Vec<String> = golden
        .lines()
        .map(|l| l.replace("$VERSION", VERSION).replace("$BUILD", BUILD))
        .collect();
    let mut call = |request: &str| {
        (&ours)
            .write_all(format!("{request}\n").as_bytes())
            .expect("write");
        let mut line = String::new();
        reader.read_line(&mut line).expect("read");
        line.trim_end().to_owned()
    };
    assert_eq!(call(&exchange[0]), exchange[1]);
    assert_eq!(
        call(r#"{"jsonrpc":"2.0","id":2,"method":"ls","params":{}}"#),
        r#"{"jsonrpc":"2.0","id":2,"result":[]}"#
    );
    ours.shutdown(std::net::Shutdown::Write).expect("shutdown");
    assert!(child.wait().expect("shed connect ends").success());
}

#[test]
fn on_a_machine_ssh_cannot_reach_fails_fast_with_the_hint() {
    let root = temp_dir("machine-down");
    script(
        &root.join("bin/ssh"),
        "echo 'ssh: connect to host down.example: Connection refused' >&2\nexit 255\n",
    );
    assert!(
        shed_on(&root, &["machine", "add", "down", "down.example"])
            .status()
            .expect("shed runs")
            .success()
    );
    let mut child = shed_on(&root, &["--on", "down", "ls"])
        .stderr(Stdio::piped())
        .spawn()
        .expect("shed runs");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while child.try_wait().expect("try_wait").is_none() {
        if std::time::Instant::now() > deadline {
            child.kill().ok();
            panic!("shed --on hung after ssh exited");
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    let out = child.wait_with_output().expect("output");
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("no answer from shed on down")
            && stderr.contains("`ssh down.example shed hello`"),
        "{stderr}"
    );
}
