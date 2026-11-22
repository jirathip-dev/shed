//! `account use`: the owner's switch moves every live lane on `auto` or `default` to the named account (the
//! session's files copied, the session loaded, `why: owner`), leaves a lane on another named account, sends the
//! go-on prompt only to a lane that was rate-limited, writes nothing under `default`'s folder, and a lane moved
//! off `default` follows `auto` on its next limit; a target that is not a ready named account is refused.
#![cfg(test)]

mod common;

use std::collections::BTreeMap;
use std::io::BufRead as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use common::{Server, temp_dir};
use serde_json::{Value, json};
use shed_client::Conn;
use shed_server::Paths;
use shed_server::proto::{EventsParams, Method};

const SHED: &str = env!("CARGO_BIN_EXE_shed");

fn shed(dir: &Path, args: &[&str]) -> Output {
    let out = Command::new(SHED).arg("--dir").arg(dir).args(args).output();
    out.expect("shed runs")
}

fn ok(dir: &Path, args: &[&str]) -> String {
    let out = shed(dir, args);
    assert!(out.status.success(), "shed {args:?} failed: {out:?}");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn json_of(dir: &Path, args: &[&str]) -> Value {
    serde_json::from_str(&ok(dir, args)).expect("JSON")
}

fn start(dir: &Path, name: &str, on: [&str; 2]) {
    let agent = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fake_acp.py");
    let history = dir.join(format!("{name}.jsonl"));
    let mut args = vec!["run", "--name", name, "--label", "run=use", "--acp", "fake"];
    args.extend(on);
    let (agent, history) = (
        agent.to_str().expect("utf-8"),
        history.to_str().expect("utf-8"),
    );
    args.extend(["--", "python3", agent, "--unique", "--history", history]);
    ok(dir, &args);
    let ready = [
        "wait",
        name,
        "--output",
        "ACP session ready",
        "--timeout",
        "10",
    ];
    ok(dir, &ready);
}

fn files(dir: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut all = BTreeMap::new();
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let path = entry.path();
        if path.is_dir() {
            all.extend(files(&path));
        } else {
            all.insert(path.clone(), std::fs::read(&path).expect("read"));
        }
    }
    all
}

struct Watch {
    events: Box<dyn Iterator<Item = std::io::Result<String>>>,
    moves: Vec<(String, Value)>,
    prompts: BTreeMap<String, Vec<String>>,
}

impl Watch {
    fn until(&mut self, want: impl Fn(&Value) -> bool) {
        loop {
            let line = self
                .events
                .next()
                .expect("an event")
                .expect("an event line");
            let event: Value = serde_json::from_str(&line).expect("JSON");
            let name = event["name"].as_str().unwrap_or_default().to_owned();
            match event["event"].as_str() {
                Some("account") => self.moves.push((name, event.clone())),
                Some("turn") => {
                    let prompt = event["prompt"].as_str().unwrap_or_default().to_owned();
                    self.prompts.entry(name).or_default().push(prompt);
                }
                _ => {}
            }
            if want(&event) {
                return;
            }
        }
    }

    fn reply(&mut self, terminal: &str) {
        self.until(|e| e["name"] == json!(terminal) && e["event"] == json!("reply"));
    }
}

fn refused(dir: &Path, name: &str) -> String {
    let out = shed(dir, &["account", "use", "fake", name]);
    assert!(!out.status.success(), "use {name} was taken");
    String::from_utf8_lossy(&out.stderr).into_owned()
}

#[test]
fn account_use_moves_the_auto_and_default_lanes_and_leaves_a_named_one() {
    let dir = temp_dir("account-use");
    let home = dir.join("default-home");
    std::fs::create_dir_all(&home).expect("default folder");
    let providers = Paths::in_dir(&dir).providers();
    std::fs::create_dir_all(&providers).expect("providers dir");
    let record = format!(
        "id = \"fake\"\nagent = \"fake\"\nconfig_dir_var = \"FAKE_HOME\"\nstatus = \"echo in\"\n\
         limit = [\"hit your limit\"]\nresets = [\"resets \"]\n\
         session_files = [\"projects/*/{{session}}.jsonl\"]\ndefault_folder = {:?}\n",
        home.to_str().expect("utf-8")
    );
    std::fs::write(providers.join("fake.toml"), record).expect("write the record");
    let _server = Server::start(&dir);
    ok(&dir, &["account", "add", "fake", "c"]);
    ok(&dir, &["account", "status", "fake", "c"]);
    start(&dir, "l1", ["--account", "auto"]);
    start(&dir, "l3", ["--account", "c"]);
    for name in ["b", "d", "e"] {
        ok(&dir, &["account", "add", "fake", name]);
    }
    ok(&dir, &["account", "status", "fake", "b"]);
    let fake_home = format!("FAKE_HOME={}", home.to_str().expect("utf-8"));
    start(&dir, "l2", ["--env", &fake_home]);
    let readings = dir.join("accounts/accounts.json");
    let mut all: Value =
        serde_json::from_slice(&std::fs::read(&readings).expect("readings")).expect("JSON");
    all["fake/e"] = json!({"state": "ready", "line": "", "limited_until": 4_000_000_000_u64});
    std::fs::write(&readings, all.to_string()).expect("mark e limited");
    for (name, code) in [
        ("default", "INVALID_PARAMS"),
        ("nope", "NO_ACCOUNT"),
        ("d", "ACCOUNT_SIGNED_OUT"),
        ("e", "ACCOUNT_LIMITED"),
    ] {
        let err = refused(&dir, name);
        assert!(err.contains(code), "use {name}: {err}");
    }

    let mut client = Conn::connect(&Paths::in_dir(&dir).socket).expect("connect");
    let params = EventsParams {
        labels: [("run".to_owned(), "use".to_owned())].into(),
        since: None,
    };
    let _: Value = client.call(Method::Events, &params).expect("events");
    let (reader, _writer) = client.split();
    let timeout = Some(std::time::Duration::from_secs(10));
    reader.get_ref().set_read_timeout(timeout).expect("timeout");
    let mut watch = Watch {
        events: Box::new(reader.lines()),
        moves: Vec::new(),
        prompts: BTreeMap::new(),
    };

    ok(&dir, &["send", "l1", "account keep"]);
    watch.reply("l1");
    ok(&dir, &["send", "l2", "account limit"]);
    watch.reply("l2");
    ok(
        &dir,
        &["wait", "l2", "--until", "rate-limited", "--timeout", "10"],
    );
    let before = files(&home);
    assert_eq!(
        before.len(),
        1,
        "l2 kept its session on default: {before:?}"
    );

    let used = json_of(&dir, &["account", "use", "fake", "b", "--json"]);
    let stayed = json!([{"terminal": "l3", "why": "on named account c"}]);
    assert_eq!(used, json!({"moved": ["l1", "l2"], "stayed": stayed}));
    watch.reply("l2");
    let mut owner: Vec<_> = (watch.moves.iter())
        .map(|(n, e)| {
            (
                n.as_str(),
                e["was"].clone(),
                e["account"].clone(),
                e["why"].clone(),
            )
        })
        .collect();
    owner.sort_by(|a, b| a.0.cmp(b.0));
    let want = [
        ("l1", json!("c"), json!("b"), json!("owner")),
        ("l2", json!("default"), json!("b"), json!("owner")),
    ];
    assert_eq!(owner, want);
    for lane in ["l1", "l2"] {
        ok(&dir, &["send", lane, "account"]);
        watch.reply(lane);
        let read = ok(&dir, &["read", lane]);
        assert!(read.contains("on b, session file: yes"), "{lane}: {read}");
        assert_eq!(json_of(&dir, &["get", lane])["account"], json!("b"));
    }
    assert_eq!(
        watch.prompts["l1"],
        ["account keep", "account"],
        "idle: no go-on"
    );
    let l2 = &watch.prompts["l2"];
    assert_eq!(l2.len(), 3, "{l2:?}");
    assert!(l2[1].contains("moved this session"), "{l2:?}");
    assert_eq!(json_of(&dir, &["get", "l3"])["account"], json!("c"));
    assert_eq!(
        files(&home),
        before,
        "nothing written under default's folder"
    );
    let listed = json_of(&dir, &["account", "ls", "fake", "--json"]);
    let preferred: Vec<_> = (listed.as_array().expect("a list").iter())
        .filter(|a| a["preferred"] == json!(true))
        .map(|a| a["name"].clone())
        .collect();
    assert_eq!(preferred, [json!("b")]);

    watch.moves.clear();
    ok(&dir, &["send", "l2", "account limit"]);
    watch.until(|e| e["name"] == json!("l2") && e["event"] == json!("account"));
    let (_, limit) = watch.moves.last().expect("the switch");
    assert_eq!(
        (&limit["was"], &limit["account"], &limit["why"]),
        (&json!("b"), &json!("c"), &json!("limit"))
    );

    ok(&dir, &["account", "use", "fake", "--clear"]);
    let listed = json_of(&dir, &["account", "ls", "fake", "--json"]);
    assert!(
        (listed.as_array().expect("a list").iter()).all(|a| a.get("preferred").is_none()),
        "{listed}"
    );
}
