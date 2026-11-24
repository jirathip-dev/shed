//! `shed secret`: shed's own secrets are set from stdin, never from an argument, and no output, log or state
//! file shows a value again; `shed doctor` names a wide file with its fix.
#![cfg(test)]

mod common;

use std::io::Write as _;
use std::os::unix::fs::PermissionsExt as _;
use std::path::Path;
use std::process::{Command, Output, Stdio};

use common::{Server, temp_dir};

const SHED: &str = env!("CARGO_BIN_EXE_shed");
const NAME: &str = "SHED_TEST_344_KEY";
const VALUE: &str = "bk-test-344-Zq7xK2mP9vL4nR8tW3yB";

fn shed(dir: &Path, args: &[&str], stdin: &str) -> (bool, String) {
    let mut child = (Command::new(SHED).arg("--dir").arg(dir).args(args))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("shed runs");
    let mut input = child.stdin.take().expect("stdin");
    input.write_all(stdin.as_bytes()).expect("write stdin");
    drop(input);
    let Output {
        status,
        stdout,
        stderr,
    } = child.wait_with_output().expect("shed ends");
    let text = String::from_utf8_lossy(&stdout).into_owned() + &String::from_utf8_lossy(&stderr);
    (status.success(), text)
}

fn files_holding(dir: &Path, value: &str) -> Vec<String> {
    let mut found = Vec::new();
    for e in std::fs::read_dir(dir).expect("read dir").flatten() {
        let path = e.path();
        if path.is_dir() {
            found.extend(files_holding(&path, value));
        } else if let Ok(bytes) = std::fs::read(&path) {
            if String::from_utf8_lossy(&bytes).contains(value) {
                found.push(path.display().to_string());
            }
        }
    }
    found
}

#[test]
fn a_secret_set_from_stdin_is_never_shown_again() {
    let dir = temp_dir("secret");
    let _server = Server::start(&dir);
    let (took_arg, arg) = shed(&dir, &["secret", "set", NAME, VALUE], "");
    assert!(
        !took_arg && arg.contains("unexpected argument"),
        "a value argument is refused: {arg}"
    );
    let (saved, set) = shed(&dir, &["secret", "set", NAME], &format!("{VALUE}\n"));
    let file = dir.join("secrets").join(NAME);
    if cfg!(target_os = "macos") {
        assert!(
            !saved && set.contains("SECRET_STORE") && set.contains("Apple-issued"),
            "{set}"
        );
    } else {
        assert!(saved, "{set}");
        assert_eq!(std::fs::read_to_string(&file).expect("the file"), VALUE);
        let mode = std::fs::metadata(&file).expect("meta").permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
        let (_, listed) = shed(&dir, &["--json", "secret", "ls"], "");
        assert!(
            listed.contains(NAME) && listed.contains("saved_at"),
            "{listed}"
        );
    }
    let mut shown = vec![set];
    for args in [
        &["secret", "ls"][..],
        &["--json", "secret", "ls"],
        &["doctor"],
    ] {
        shown.push(shed(&dir, args, "").1);
    }
    for _ in 0..2 {
        let (removed, rm) = shed(&dir, &["secret", "rm", NAME], "");
        assert!(removed, "twice is once: {rm}");
        shown.push(rm);
    }
    assert!(!file.exists());
    for text in &shown {
        assert!(!text.contains(VALUE), "{text}");
    }
    assert_eq!(files_holding(&dir, VALUE), Vec::<String>::new());

    let wide = dir.join("secrets").join("WIDE");
    std::fs::create_dir_all(dir.join("secrets")).expect("secrets dir");
    std::fs::write(&wide, VALUE).expect("wide file");
    std::fs::set_permissions(&wide, std::fs::Permissions::from_mode(0o644)).expect("chmod");
    let (healthy, report) = shed(&dir, &["doctor"], "");
    let fix = format!(
        "FAIL  secrets  {} is wider than 0600: not read; fix: chmod 600 ",
        wide.display()
    );
    assert!(!healthy && report.contains(&fix), "{report}");
    assert!(!report.contains(VALUE), "{report}");
}
