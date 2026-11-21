use std::io::Write as _;
use std::os::unix::fs::PermissionsExt as _;
use std::path::Path;
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::common::{self, temp_dir};
use crate::{box_ssh, linked, shed_on};

fn fed(root: &Path, args: &[&str], input: &[u8]) -> Output {
    let mut child = shed_on(root, args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("shed runs");
    let mut stdin = child.stdin.take().expect("stdin");
    stdin.write_all(input).expect("write stdin");
    drop(stdin);
    child.wait_with_output().expect("shed ends")
}

fn gone(pid_file: &Path) {
    let pid = std::fs::read_to_string(pid_file).expect("the child's pid");
    common::eventually("the background child to end", || {
        let alive = Command::new("kill")
            .args(["-0", pid.trim()])
            .stderr(Stdio::null())
            .status();
        !alive.expect("kill runs").success()
    });
}

fn exec_and_files(root: &Path, on: &[&str]) {
    let dir = root.join("remote");
    let shed = |args: &[&str], input: &[u8]| fed(root, &[on, args].concat(), input);
    let refused = |out: &Output, why: &str| {
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(!out.status.success() && out.stdout.is_empty(), "{out:?}");
        assert!(stderr.contains("IO: ") && stderr.contains(why), "{stderr}");
    };
    let script = r"printf 'a\377\0b'; printf 'e\376' >&2; exit 7";
    let out = shed(&["exec", "--", "sh", "-c", script], b"");
    assert_eq!(out.stdout, b"a\xff\0b");
    assert_eq!(out.stderr, b"e\xfe");
    assert_eq!(out.status.code(), Some(7));
    assert_eq!(
        shed(&["exec", "-i", "--", "cat"], b"in\xff\0").stdout,
        b"in\xff\0"
    );

    let returned = dir.join(format!("returned{}", on.len()));
    let script = format!("sleep 60 & echo $! > '{}'", returned.display());
    let started = Instant::now();
    assert!(
        shed(&["exec", "--", "sh", "-c", &script], b"")
            .status
            .success()
    );
    assert!(started.elapsed() < Duration::from_secs(10));
    gone(&returned);
    let timed_out = dir.join(format!("timed-out{}", on.len()));
    let script = format!("sleep 60 & echo $! > '{}'; wait", timed_out.display());
    let out = shed(
        &["exec", "--timeout", "0.5", "--", "sh", "-c", &script],
        b"",
    );
    refused(&out, "timed out");
    gone(&timed_out);

    let file = dir.join(format!("blob{}", on.len()));
    let path = file.to_str().expect("utf-8 path");
    std::fs::write(&file, b"old").expect("write");
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o640)).expect("chmod");
    let bytes: Vec<u8> = (0..=255).chain((0..=255).rev()).collect();
    assert!(shed(&["file", "write", path], &bytes).status.success());
    let mode = std::fs::metadata(&file)
        .expect("written")
        .permissions()
        .mode();
    assert_eq!(
        (std::fs::read(&file).expect("read"), mode & 0o777),
        (bytes.clone(), 0o640)
    );
    assert_eq!(shed(&["file", "read", path], b"").stdout, bytes);
    outline_and_lines(&dir, &shed);

    let big = dir.join(format!("big{}", on.len()));
    let file = std::fs::File::create(&big).expect("create");
    file.set_len((8 << 20) + 1).expect("grow");
    let path = big.to_str().expect("utf-8 path");
    refused(&shed(&["file", "read", path], b""), "over the reply limit");
    let flood = ["exec", "--", "head", "-c", "8388609", "/dev/zero"];
    refused(&shed(&flood, b""), "over the reply limit");
    let most = ["exec", "--", "head", "-c", "8388608", "/dev/zero"];
    assert_eq!(shed(&most, b"").stdout.len(), 8 << 20);
}

fn outline_and_lines(dir: &Path, shed: &dyn Fn(&[&str], &[u8]) -> Output) {
    let file = dir.join("job.rs");
    let path = file.to_str().expect("utf-8 path");
    let source = "struct Job {\n    id: u64,\n}\n\nimpl Job {\n    fn run(&self) {}\n}\n";
    std::fs::write(&file, source).expect("write");
    let text = shed(&["code", "outline", path], b"");
    let want = format!(
        "{path} (rust, 7 lines, 3 symbols)\n  struct Job [1-3]\n  impl Job [5-7]\n    fn run [6-6]\n"
    );
    assert_eq!(String::from_utf8_lossy(&text.stdout), want, "{text:?}");
    let json = shed(&["--json", "code", "outline", path], b"").stdout;
    let json: Value = serde_json::from_slice(&json).expect("json");
    assert_eq!(
        json["symbols"][2],
        json!({"kind": "fn", "name": "run", "start": 6, "end": 6, "depth": 1})
    );
    assert_eq!(
        (json["lang"].as_str(), json["lines"].as_u64()),
        (Some("rust"), Some(7))
    );
    let span = shed(&["file", "read", path, "--lines", "2-3"], b"").stdout;
    assert_eq!(span, b"    id: u64,\n}\n");
    assert_eq!(
        shed(&["file", "read", path, "--lines", "7"], b"").stdout,
        b"}\n"
    );
    assert!(
        shed(&["file", "read", path, "--lines", "8-9"], b"")
            .stdout
            .is_empty()
    );
    let missing = shed(&["code", "outline", &format!("{path}.gone")], b"");
    let stderr = String::from_utf8_lossy(&missing.stderr);
    assert!(
        !missing.status.success() && stderr.contains("IO: ") && stderr.contains(".gone"),
        "{stderr}"
    );
}

#[test]
fn exec_and_file_return_exact_bytes_and_codes_here_and_over_the_held_link() {
    let root = temp_dir("exec");
    let (_remote, _local) = linked(&root);
    exec_and_files(&root, &[]);
    let ssh = box_ssh(&root);
    exec_and_files(&root, &["--on", "box"]);
    assert_eq!(box_ssh(&root), ssh, "no ssh per call while the link is up");
}
