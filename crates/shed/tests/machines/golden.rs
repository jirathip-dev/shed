use std::fmt::Write as _;
use std::path::Path;
use std::process::{Command, Output};

use shed_server::proto::BUILD;

use crate::common::{Server, temp_dir};
use crate::{SHED, VERSION, fake_hosts, shed_on};

fn record(root: &Path, text: &mut String, args: &[&str]) -> Output {
    let out = shed_on(root, args).output().expect("shed runs");
    write!(
        text,
        "$ shed {}\n{}",
        args.join(" "),
        String::from_utf8_lossy(&out.stdout)
    )
    .ok();
    if !out.status.success() {
        let code = out.status.code().unwrap_or(-1);
        write!(
            text,
            "[exit {code}] {}",
            String::from_utf8_lossy(&out.stderr)
        )
        .ok();
    }
    out
}

#[test]
fn on_a_machine_run_read_kill_and_ls_reach_the_remote_server_and_versions_must_match() {
    let root = temp_dir("machines");
    let bin = fake_hosts(&root);
    let remote = root.join("remote");
    let _server = Server::start(&remote);
    let remote_arg = remote.to_str().expect("utf-8 dir");
    let mut text = String::new();
    for args in [
        &["machine", "ls"][..],
        &["machine", "add", "box", "user@box.example"],
        &["machine", "add", "old", "old.example"],
        &["machine", "add", "box", "other.example"],
        &["machine", "add", "bad", "--", "-oProxyCommand=x"],
        &["--json", "machine", "ls"],
        &["machine", "ls"],
        &["--on", "nope", "ls"],
        &["--on", "box", "serve"],
        &["--on", "box", "run", "--", "true"],
        &[
            "--on",
            "box",
            "run",
            "--name",
            "t1",
            "--cwd",
            remote_arg,
            "--",
            "sh",
            "-c",
            "echo hello from box; exec sleep 100",
        ],
        &["--on", "box", "wait", "t1", "--output", "hello from box"],
        &["--on", "box", "read", "t1"],
        &["--on", "box", "kill", "t1"],
        &["--on", "box", "ls"],
        &["--on", "old", "ls"],
        &["machine", "rm", "old"],
        &["machine", "rm", "old"],
        &["machine", "ls"],
    ] {
        record(&root, &mut text, args);
    }
    let direct = Command::new(SHED)
        .arg("--dir")
        .arg(&remote)
        .arg("ls")
        .output()
        .expect("shed ls");
    assert!(String::from_utf8_lossy(&direct.stdout).contains("t1"));
    let log = std::fs::read_to_string(bin.join("ssh.log")).expect("ssh ran");
    for option in [
        "-o BatchMode=yes",
        "-o ConnectTimeout=10",
        "-o ControlMaster=auto",
        "-- user@box.example shed connect",
    ] {
        assert!(log.contains(option), "{option} missing from {log}");
    }
    let text = text
        .replace(root.to_str().expect("utf-8 dir"), "$DIR")
        .replace(VERSION, "$VERSION")
        .replace(BUILD, "$BUILD");
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden/machines.txt");
    if std::env::var_os("SHED_BLESS").is_some() {
        std::fs::write(&path, &text).expect("write golden");
    }
    let have = std::fs::read_to_string(&path).unwrap_or_default();
    assert_eq!(have, text, "machines.txt differs; SHED_BLESS=1 to update");
}
