use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use shed_server::Paths;
use shed_server::proto::BUILD;

use crate::common::{Server, eventually, temp_dir};
use crate::{Conn, VERSION, ours};

struct Goldens {
    dir: PathBuf,
    files: Vec<(String, String)>,
}

impl Goldens {
    fn record(&mut self, file: &str, (request, response, _): &(String, String, Value)) {
        let normal = |s: &str| {
            let s = s
                .replace(self.dir.to_str().expect("utf-8 dir"), "$DIR")
                .replace(&format!("\"{VERSION}\""), "\"$VERSION\"")
                .replace(&format!("\"{BUILD}\""), "\"$BUILD\"")
                .replace(&format!("server {BUILD}:"), "server $BUILD:");
            let mut out = String::new();
            let mut rest = s.as_str();
            while let Some(at) = rest.find("\"pid\":") {
                let (head, tail) = rest.split_at(at + 6);
                out.push_str(head);
                let digits =
                    tail.len() - tail.trim_start_matches(|c: char| c.is_ascii_digit()).len();
                out.push_str(if digits > 0 { "0" } else { "" });
                rest = &tail[digits..];
            }
            out + rest
        };
        let text = format!("{}\n{}\n", normal(request), normal(response));
        match self.files.iter_mut().find(|(name, _)| name == file) {
            Some((_, body)) => body.push_str(&text),
            None => self.files.push((file.to_owned(), text)),
        }
    }

    fn check(&self) {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden");
        let bless = std::env::var_os("SHED_BLESS").is_some();
        for (name, want) in &self.files {
            let path = root.join(format!("{name}.jsonl"));
            if bless {
                std::fs::create_dir_all(&root).expect("golden dir");
                std::fs::write(&path, want).expect("write golden");
            }
            let have = std::fs::read_to_string(&path).unwrap_or_default();
            assert_eq!(
                &have,
                want,
                "{} differs; SHED_BLESS=1 to update",
                path.display()
            );
        }
    }
}

fn record_events(dir: &Path, goldens: &mut Goldens) {
    let mut events = Conn::open(dir);
    events.result("hello", &ours());
    let (request, response, _) = events.call(22, "events", &json!({"since": 0}));
    let replay: Vec<String> = (0..4).map(|_| events.next()).collect();
    let response = format!("{response}\n{}", replay.join("\n"));
    goldens.record("events", &(request, response, Value::Null));
}

fn record_child(c: &mut Conn, goldens: &mut Goldens, dir: &Path) -> Value {
    let other = json!({"name": "other", "labels": {"role": "rev"}, "cwd": dir, "cmd": ["sleep", "100"],
        "parent": {"name": "build", "outlive": true}});
    goldens.record("run", &c.call(23, "run", &other));
    let ls = c.call(3, "ls", &json!({"labels": {"role": "impl"}}));
    goldens.record("ls", &ls);
    assert_eq!(ls.2["result"].as_array().map(Vec::len), Some(1));
    let children = c.call(24, "ls", &json!({"parent": "build"}));
    goldens.record("ls", &children);
    assert_eq!(children.2["result"][0]["name"], "other");
    other
}

#[test]
fn run_ls_read_kill_over_the_socket_match_the_goldens() {
    let dir = temp_dir("golden");
    let _server = Server::start(&dir);
    let mut goldens = Goldens {
        dir: dir.clone(),
        files: Vec::new(),
    };
    let mut c = Conn::open(&dir);
    let socket_mode = std::fs::metadata(Paths::in_dir(&dir).socket)
        .expect("socket")
        .permissions()
        .mode();
    assert_eq!(socket_mode & 0o777, 0o600);

    let first = c.call(1, "ls", &json!({}));
    goldens.record("errors", &first);
    assert_eq!(first.2["error"]["code"], -32001);

    goldens.record("hello", &c.call(1, "hello", &ours()));
    let run = c.call(
        2,
        "run",
        &json!({
            "name": "build",
            "labels": {"issue": "7", "role": "impl"},
            "cwd": dir,
            "cmd": ["sh", "-c", "printf 'hello\\n'; exec sleep 100"],
        }),
    );
    goldens.record("run", &run);
    assert_eq!(run.2["result"]["state"], "running");

    let other = record_child(&mut c, &mut goldens, &dir);

    goldens.record("get", &c.call(4, "get", &json!({"name": "build"})));
    eventually("hello on screen", || {
        c.result("read", &json!({"name": "build"}))["text"] == "hello"
    });
    let read = c.call(5, "read", &json!({"name": "build"}));
    goldens.record("read", &read);
    assert_eq!(read.2["result"]["text"], "hello");

    goldens.record(
        "send",
        &c.call(6, "send", &json!({"name": "build", "text": "x"})),
    );
    goldens.record(
        "keys",
        &c.call(7, "keys", &json!({"name": "build", "keys": ["enter"]})),
    );

    goldens.record(
        "state",
        &c.call(
            16,
            "state",
            &json!({"name": "build", "state": "working", "note": "step 1"}),
        ),
    );
    goldens.record(
        "wait",
        &c.call(
            17,
            "wait",
            &json!({"name": "build", "until": ["working"], "timeout": 10}),
        ),
    );
    goldens.record(
        "wait",
        &c.call(
            18,
            "wait",
            &json!({"name": "build", "until": ["done"], "timeout": 0.1}),
        ),
    );

    for (id, method, params) in [
        (8, "nope", json!({"name": "build"})),
        (19, "state", json!({"name": "build", "state": "idle"})),
        (20, "wait", json!({"name": "build"})),
        (21, "wait", json!({"name": "build", "output": "[a"})),
        (9, "get", json!({"name": "nope"})),
        (10, "rm", json!({"name": "build"})),
        (11, "run", other.clone()),
        (12, "keys", json!({"name": "build", "keys": ["hyper"]})),
        (13, "run", json!({"cwd": dir, "cmd": []})),
        (
            25,
            "run",
            json!({"cwd": dir, "cmd": ["true"], "parent": {"name": "nope"}}),
        ),
    ] {
        goldens.record("errors", &c.call(id, method, &params));
    }
    goldens.record("errors", &(String::from("{"), c.line("{"), Value::Null));

    let kill = c.call(14, "kill", &json!({"name": "build"}));
    goldens.record("kill", &kill);
    assert_eq!(kill.2["result"]["state"], "exited");
    assert_eq!(kill.2["result"]["exit"], 143);
    let rm = c.call(15, "rm", &json!({"name": "build"}));
    goldens.record("rm", &rm);
    let left = c.result("ls", &json!({}));
    assert_eq!(left.as_array().map(Vec::len), Some(1));

    record_events(&dir, &mut goldens);
    goldens.check();
}

#[test]
fn read_since_a_cursor_returns_only_what_is_new_and_matches_the_goldens() {
    let dir = temp_dir("since");
    let _server = Server::start(&dir);
    let mut goldens = Goldens {
        dir: dir.clone(),
        files: Vec::new(),
    };
    let mut c = Conn::open(&dir);
    c.result("hello", &ours());
    let script = "stty -echo; printf 'a\\n'; read x; printf 'b\\nc\\nd\\ne\\n'; read x; seq 5003; \
        read x; printf '\\033[?1049h\\033[H\\033[2Jtop 1'; read x; printf '\\033[Htop 1'; \
        touch redrawn; exec sleep 100";
    let run =
        json!({"name": "pager", "cwd": dir, "rows": 3, "cols": 20, "cmd": ["sh", "-c", script]});
    c.result("run", &run);
    let pager = json!({"name": "pager"});
    eventually("a", || c.result("read", &pager)["text"] == "a");
    let mut cursor = c.result("read", &pager)["cursor"].clone();
    let mut since = |c: &mut Conn, id: u64| {
        let read = c.call(id, "read", &json!({"name": "pager", "since": cursor}));
        goldens.record("read-since", &read);
        cursor = read.2["result"]["cursor"].clone();
        read.2["result"].clone()
    };
    let nothing = since(&mut c, 1);
    assert_eq!(
        (&nothing["text"], &nothing["changed"]),
        (&json!(""), &Value::Null)
    );
    for (id, screen) in [(2, "d\ne"), (3, "5002\n5003"), (4, "top 1")] {
        c.result("keys", &json!({"name": "pager", "keys": ["enter"]}));
        eventually(screen, || c.result("read", &pager)["text"] == screen);
        let new = since(&mut c, id);
        assert_eq!(new["changed"], true);
        assert_eq!(new["lost"], if id == 3 { json!(3) } else { Value::Null });
        assert!(new["text"].as_str().is_some_and(|t| t.ends_with(screen)));
    }
    c.result("keys", &json!({"name": "pager", "keys": ["enter"]}));
    eventually("the redraw", || dir.join("redrawn").exists());
    let same = since(&mut c, 5);
    assert_eq!(
        (&same["text"], &same["changed"]),
        (&json!(""), &Value::Null)
    );
    goldens.check();
}
