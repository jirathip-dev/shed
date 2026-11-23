use std::io::{BufRead as _, BufReader, Read as _, Write as _};
use std::os::unix::fs::PermissionsExt as _;
use std::path::Path;
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};
use shed_server::Paths;

use crate::common::{self, Server, temp_dir};
use crate::{SHED, fake_hosts, fault, link, ok, script, shed_on};

const TOKENS: [(&str, &str); 2] = [
    ("spare", "sk-fake-oat01-spare-0123456789abcdef"),
    ("work", "sk-fake-oat01-work-0123456789abcdef"),
];

type Seen = Arc<Mutex<Vec<String>>>;

fn upstream() -> (u16, Seen) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bound");
    let port = listener.local_addr().expect("addr").port();
    let seen = Seen::default();
    let kept = Arc::clone(&seen);
    std::thread::spawn(move || {
        for mut conn in listener.incoming().map_while(Result::ok) {
            let mut reader = BufReader::new(conn.try_clone().expect("clone"));
            let (mut head, mut length) = (String::new(), 0);
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).expect("a line");
                if let Some(n) = line.to_lowercase().strip_prefix("content-length:") {
                    length = n.trim().parse().expect("a length");
                }
                if line.trim().is_empty() {
                    break;
                }
                head += &line;
            }
            reader.read_exact(&mut vec![0; length]).expect("the body");
            kept.lock().expect("seen").push(head.to_lowercase());
            let answer = "HTTP/1.1 200 OK\r\ncontent-length: 8\r\n\r\nupstream";
            conn.write_all(answer.as_bytes()).expect("answered");
        }
    });
    (port, seen)
}

fn keys(root: &Path) -> Vec<Value> {
    let list: Value = serde_json::from_str(&ok(root, &["--json", "proxy", "keys"])).expect("JSON");
    list.as_array().expect("a list").clone()
}

fn record(dir: &Path, text: &str) {
    let providers = Paths::in_dir(dir).providers();
    std::fs::create_dir_all(&providers).expect("providers dir");
    std::fs::write(providers.join("fake.toml"), text).expect("write the record");
}

fn lab(root: &Path, port: u16) -> [Server; 2] {
    let (local, remote) = (root.join("local"), root.join("remote"));
    record(
        &local,
        &format!(
            "id = \"fake\"\nagent = \"fake\"\nconfig_dir_var = \"FAKE_HOME\"\n[proxy]\n\
             upstream = \"http://127.0.0.1:{port}\"\npaths = [\"/v1/\"]\nbase_url_var = \"FAKE_BASE_URL\"\n\
             key_var = \"FAKE_KEY\"\nauth = \"authorization: Bearer {{token}}\"\n"
        ),
    );
    record(
        &remote,
        "id = \"fake\"\nagent = \"fake\"\nlimit = [\"hit your limit\"]\nresets = [\"resets \"]\n",
    );
    fake_hosts(root);
    let sent = root.join("bin/sent.bytes");
    script(
        &root.join("user@box.example/shed"),
        &format!(
            "tee -a '{}' | '{SHED}' --dir '{}' \"$@\"\n",
            sent.display(),
            remote.display()
        ),
    );
    let agent = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fake_acp.py");
    let installed = json!({"agents": [{"id": "fake", "version": "0.1.0", "distribution": "binary",
        "cmd": ["python3", agent]}]});
    let remote_server = Server::start(&remote);
    std::fs::write(remote.join("agents.json"), installed.to_string()).expect("agents.json");
    ok(root, &["machine", "add", "box", "user@box.example"]);
    let path = format!(
        "{}:{}",
        root.join("bin").display(),
        std::env::var("PATH").unwrap()
    );
    let local_server = Server::with_env(&local, "3600", &[("PATH", &path)]);
    for (name, token) in TOKENS {
        let file = root.join(format!("{name}.token"));
        std::fs::write(&file, token).expect("write the token");
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o600)).expect("chmod");
        let file = file.to_str().expect("utf-8");
        ok(
            root,
            &["account", "add", "fake", name, "--token-file", file],
        );
        std::fs::remove_file(file).expect("rm the token");
    }
    common::eventually("the forward to box", || link(root, "box")["proxy"].is_u64());
    [local_server, remote_server]
}

#[test]
fn a_run_on_a_machine_gets_a_key_never_the_token_and_the_key_moves_on_a_limit_and_ends_with_it() {
    let root = temp_dir("link-proxy");
    let (port, seen) = upstream();
    let _servers = lab(&root, port);
    let cwd = root.join("remote");
    let cwd = cwd.to_str().expect("utf-8 dir");
    let run = [
        "--on",
        "box",
        "run",
        "--name",
        "a",
        "--acp",
        "fake",
        "--account",
        "auto",
        "--cwd",
        cwd,
    ];
    assert_eq!(ok(&root, &run), "box/a\n");
    let ready = [
        "wait",
        "box/a",
        "--output",
        "ACP session ready",
        "--timeout",
        "10",
    ];
    assert!(
        shed_on(&root, &ready)
            .output()
            .expect("wait")
            .status
            .success()
    );
    let key = &keys(&root)[0];
    assert_eq!(
        (&key["terminal"], &key["state"]),
        (&json!("box/a"), &json!("live")),
        "{key}"
    );
    let first = key["account"].as_str().expect("an account").to_owned();
    let token = |name: &str| TOKENS.iter().find(|t| t.0 == name).expect("a token").1;

    let proxied = |n: usize| {
        common::eventually("the proxied reply", || {
            ok(&root, &["read", "box/a"])
                .matches("proxied 200: upstream")
                .count()
                == n
        });
    };
    ok(&root, &["send", "box/a", "proxy /v1/messages"]);
    proxied(1);
    let head = seen.lock().expect("seen")[0].clone();
    let bearer = format!("authorization: bearer {}", token(&first).to_lowercase());
    assert!(head.contains(&bearer), "{head}");
    assert!(
        !head.contains("x-api-key") && !head.contains("stale-of-its-own") && !head.contains("shk-"),
        "{head}"
    );

    ok(&root, &["send", "box/a", "proxy limit"]);
    common::eventually("the key on the next account", || {
        keys(&root)[0]["account"] != first.as_str()
    });
    let next = keys(&root)[0]["account"]
        .as_str()
        .expect("an account")
        .to_owned();
    let accounts: Value =
        serde_json::from_str(&ok(&root, &["--json", "account", "ls", "fake"])).expect("JSON");
    let marked = accounts
        .as_array()
        .expect("a list")
        .iter()
        .find(|a| a["name"] == first.as_str());
    assert_eq!(
        marked.expect("the first account")["state"],
        "limited",
        "{accounts}"
    );
    common::eventually("the go-on prompt", || {
        ok(&root, &["read", "box/a"]).contains("shed moved this session")
    });
    ok(&root, &["send", "box/a", "proxy /v1/messages"]);
    proxied(2);
    let head = seen.lock().expect("seen")[1].clone();
    assert!(
        head.contains(&format!(
            "authorization: bearer {}",
            token(&next).to_lowercase()
        )),
        "{head}"
    );

    let secret = no_token_sent(&root);
    ended_key_is_refused(&root, &seen, &secret);
}

fn no_token_sent(root: &Path) -> String {
    let sent = std::fs::read(root.join("bin/sent.bytes")).expect("the machine's bytes");
    let sent = String::from_utf8_lossy(&sent);
    let at = sent
        .find("shk-")
        .expect("the run's key went to the machine");
    let secret = sent[at..at + 52].to_owned();
    for (_, token) in TOKENS {
        assert!(!sent.contains(token), "a token reached the machine");
    }

    secret
}

fn ended_key_is_refused(root: &Path, seen: &Seen, secret: &str) {
    assert_eq!(ok(root, &["kill", "box/a"]), "box/a exited 143\n");
    common::eventually("the key ended with its terminal", || {
        keys(root)[0]["state"] == "ended"
    });
    let relay = link(root, "box")["proxy"].as_u64().expect("the relay port");
    let mut call = std::net::TcpStream::connect(("127.0.0.1", u16::try_from(relay).unwrap()))
        .expect("the relay");
    let request =
        format!("POST /v1/messages HTTP/1.1\r\nx-api-key: {secret}\r\ncontent-length: 0\r\n\r\n");
    call.write_all(request.as_bytes()).expect("sent");
    let mut answer = String::new();
    call.read_to_string(&mut answer).expect("an answer");
    assert!(answer.starts_with("HTTP/1.1 401"), "{answer}");
    assert_eq!(
        seen.lock().expect("seen").len(),
        2,
        "an ended key never reaches upstream"
    );
}

#[test]
#[ignore = "TODO(#309): a key ends when one link's ls misses its terminal"]
fn a_key_ends_with_its_terminal_not_with_one_links_guess() {
    let root = temp_dir("symptom-key");
    let (port, _seen) = upstream();
    let _servers = lab(&root, port);
    let cwd = root.join("remote");
    let cwd = cwd.to_str().expect("utf-8 dir");
    let run = [
        "--on",
        "box",
        "run",
        "--name",
        "a",
        "--acp",
        "fake",
        "--account",
        "auto",
    ];
    assert_eq!(ok(&root, &[&run[..], &["--cwd", cwd]].concat()), "box/a\n");
    common::eventually("a live key", || keys(&root)[0]["state"] == "live");
    let moved = root.join("moved");
    let _moved = Server::start(&moved);
    let exec = format!("exec '{SHED}' --dir '{}' \"$@\"\n", moved.display());
    script(&root.join("new.example/shed"), &exec);
    fault(&root, "user@box.example cut\n");
    common::eventually("the link cut", || link(&root, "box")["state"] != "up");
    fault(&root, "user@box.example moved new.example\n");
    common::eventually("the link at the new address", || {
        link(&root, "box")["state"] == "up"
    });
    let there = std::process::Command::new(SHED)
        .arg("--dir")
        .arg(root.join("remote"))
        .args(["get", "a"])
        .output();
    assert!(String::from_utf8_lossy(&there.expect("shed get").stdout).contains("running"));
    assert_eq!(
        keys(&root)[0]["state"],
        "live",
        "a still runs where it started"
    );
}
