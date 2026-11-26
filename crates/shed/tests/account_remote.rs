//! `account login --remote`: no browser opens on the host, the sign-in link comes back unredacted while the
//! login's screen stays redacted, and the answer goes back the record's way: pasted, fetched as a localhost
//! callback, or none for a device-code login.
#![cfg(test)]

mod common;

use std::io::{BufRead as _, BufReader, Read as _, Write as _};
use std::net::TcpListener;
use std::path::Path;
use std::process::{Command, Output, Stdio};

use common::{Server, temp_dir};
use shed_server::Paths;

const SHED: &str = env!("CARGO_BIN_EXE_shed");
const STATE: &str = "Zq7xK2mP9vL4nR8tW3yB6cD1fG5hJ0kA";
const WAIT_IN: &str = "while [ ! -f \\\"$FAKE_HOME/in\\\" ]; do sleep 0.1; done";

fn record(id: &str, login: &str, remote: &str) -> String {
    format!(
        "id = \"{id}\"\nagent = \"{id}\"\nconfig_dir_var = \"FAKE_HOME\"\n\
         status = \"test -f \\\"$FAKE_HOME/in\\\" && echo signed in\"\nlogin = \"{login}\"\n\
         [remote_login]\n{remote}"
    )
}

fn setup(name: &str, records: &[(&str, String)]) -> (std::path::PathBuf, Server) {
    let dir = temp_dir(name);
    let providers = Paths::in_dir(&dir).providers();
    std::fs::create_dir_all(&providers).expect("providers dir");
    for (id, text) in records {
        std::fs::write(providers.join(format!("{id}.toml")), text).expect("write the record");
    }
    let server = Server::start(&dir);
    for (id, _) in records {
        shed(&dir, &["account", "add", id, "work"], "");
    }
    (dir, server)
}

fn shed(dir: &Path, args: &[&str], stdin: &str) -> Output {
    let mut child = Command::new(SHED)
        .arg("--dir")
        .arg(dir)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("shed runs");
    let mut input = child.stdin.take().expect("stdin");
    input.write_all(stdin.as_bytes()).expect("the answer");
    drop(input);
    child.wait_with_output().expect("shed ends")
}

fn text(out: &Output) -> String {
    let (stdout, stderr) = (&out.stdout, &out.stderr);
    format!(
        "{}{}",
        String::from_utf8_lossy(stdout),
        String::from_utf8_lossy(stderr)
    )
}

#[test]
fn a_pasted_code_signs_in_and_the_link_comes_back_with_no_browser_opened() {
    let link = format!("https://fake.example/oauth?token={STATE}");
    let login = format!(
        "open '{link}'; read c; [ \\\"$c\\\" = right-code ] && touch \\\"$FAKE_HOME/in\\\""
    );
    let remote = "link = \"https://fake.example/oauth\"\nanswer = \"paste\"\n";
    let (dir, _server) = setup(
        "remote-paste",
        &[("paste", record("paste", &login, remote))],
    );
    let out = shed(
        &dir,
        &["account", "login", "paste", "work", "--remote"],
        "right-code\n",
    );
    let shown = text(&out);
    assert!(out.status.success(), "{shown}");
    assert!(
        shown.contains(&format!("Open on any device: {link}")),
        "{shown}"
    );
    assert!(shown.contains("ready"), "{shown}");
    assert!(dir.join("accounts/paste/work/in").exists(), "{shown}");
    let read = text(&shed(&dir, &["read", "login-paste-work"], ""));
    assert!(
        read.contains("[redacted]") && !read.contains(STATE),
        "{read}"
    );
}

#[test]
fn a_callback_address_is_fetched_on_the_host_and_only_a_localhost_one() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("a port");
    let port = listener.local_addr().expect("addr").port();
    let login = format!(
        "xdg-open 'https://accounts.fake.example/auth?redirect_uri=http://127.0.0.1:{port}/cb'; {WAIT_IN}"
    );
    let remote = "link = \"https://accounts.fake.example/\"\nanswer = \"callback\"\n";
    let (dir, _server) = setup(
        "remote-callback",
        &[("callback", record("callback", &login, remote))],
    );
    let args = ["account", "login", "callback", "work", "--remote"];
    let refused = shed(&dir, &args, "https://evil.example/cb?code=abc\n");
    assert!(!refused.status.success(), "{}", text(&refused));
    assert!(
        text(&refused).contains("http://localhost:"),
        "{}",
        text(&refused)
    );
    let home = dir.join("accounts/callback/work");
    let served = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("the callback");
        let mut line = String::new();
        BufReader::new(stream.try_clone().expect("clone"))
            .read_line(&mut line)
            .expect("the request line");
        std::fs::write(home.join("in"), &line).expect("signed in");
        stream
            .write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 0\r\nconnection: close\r\n\r\n")
            .expect("the reply");
        let mut rest = [0; 1024];
        stream.read(&mut rest).ok();
    });
    let answer = format!("http://127.0.0.1:{port}/cb?code=abc\n");
    let out = shed(&dir, &args, &answer);
    let shown = text(&out);
    assert!(out.status.success(), "{shown}");
    assert!(
        shown.contains("https://accounts.fake.example/auth?redirect_uri="),
        "{shown}"
    );
    assert!(shown.contains("ready"), "{shown}");
    served.join().expect("the listener");
    let got = std::fs::read_to_string(dir.join("accounts/callback/work/in")).expect("in");
    assert!(got.starts_with("GET /cb?code=abc "), "{got}");
}

#[test]
fn a_device_code_login_shows_the_code_and_takes_no_answer() {
    let login = format!(
        "printf 'Open https://fake.example/device\\\\n\\\\nEnter this one-time code (soon)\\\\n   ABCD-1234\\\\n'; {WAIT_IN}"
    );
    let remote =
        "link = \"https://fake.example/device\"\ncode = \"one-time code\"\nanswer = \"none\"\n";
    let (dir, _server) = setup(
        "remote-device",
        &[("device", record("device", &login, remote))],
    );
    let mut child = Command::new(SHED)
        .arg("--dir")
        .arg(&dir)
        .args(["account", "login", "device", "work", "--remote"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .spawn()
        .expect("shed runs");
    let mut lines = BufReader::new(child.stdout.take().expect("stdout")).lines();
    let mut seen = Vec::new();
    for line in lines.by_ref() {
        let line = line.expect("a line");
        let done = line.contains("Enter this code there: ABCD-1234");
        seen.push(line);
        if done {
            break;
        }
    }
    assert!(
        seen.iter()
            .any(|l| l.contains("https://fake.example/device")),
        "{seen:?}"
    );
    let answer = shed(&dir, &["account", "answer", "device", "work", "x"], "");
    assert!(!answer.status.success(), "{}", text(&answer));
    std::fs::write(dir.join("accounts/device/work/in"), "").expect("approved");
    let rest: Vec<String> = lines.map_while(Result::ok).collect();
    assert!(child.wait().expect("shed ends").success(), "{rest:?}");
    assert!(rest.iter().any(|l| l.contains("ready")), "{rest:?}");
}
