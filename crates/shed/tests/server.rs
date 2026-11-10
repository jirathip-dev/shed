//! `shed serve` end to end: a real server in a temp dir, driven over its socket.
//!
//! Every method's request and response is checked against `tests/golden/<method>.jsonl` (the temp
//! dir, version and pids normalised). Regenerate with `SHED_BLESS=1 cargo test -p shed`.
#![cfg(test)]

mod common;

use std::io::{BufRead as _, BufReader, Write as _};
use std::os::unix::fs::PermissionsExt as _;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};

use common::{Server, eventually, temp_dir};
use serde_json::{Value, json};
use shed_server::proto::{Method, ReadParams, ReadResult, SendParams, State, TerminalInfo};
use shed_server::{Client, Paths};

const VERSION: &str = env!("CARGO_PKG_VERSION");

struct Conn {
    reader: BufReader<UnixStream>,
    writer: UnixStream,
}

impl Conn {
    fn open(dir: &Path) -> Self {
        let writer = UnixStream::connect(Paths::in_dir(dir).socket).expect("connect");
        Self {
            reader: BufReader::new(writer.try_clone().expect("clone")),
            writer,
        }
    }

    fn line(&mut self, request: &str) -> String {
        self.writer
            .write_all(format!("{request}\n").as_bytes())
            .expect("write");
        let mut reply = String::new();
        self.reader.read_line(&mut reply).expect("read");
        reply.trim_end().to_owned()
    }

    fn call(&mut self, id: u64, method: &str, params: &Value) -> (String, String, Value) {
        let request = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
        let request = request.to_string();
        let response = self.line(&request);
        let value = serde_json::from_str(&response).expect("response is JSON");
        (request, response, value)
    }

    fn next(&mut self) -> String {
        self.reader
            .get_ref()
            .set_read_timeout(Some(std::time::Duration::from_secs(10)))
            .expect("timeout");
        let mut line = String::new();
        self.reader.read_line(&mut line).expect("an event line");
        line.trim_end().to_owned()
    }

    fn event(&mut self) -> Value {
        serde_json::from_str(&self.next()).expect("event is JSON")
    }

    fn events(dir: &Path, params: &Value) -> Self {
        let mut c = Self::open(dir);
        c.result("hello", &json!({"product": "shed", "version": VERSION}));
        assert_eq!(c.result("events", params), json!({}));
        c
    }

    fn result(&mut self, method: &str, params: &Value) -> Value {
        let (_, line, response) = self.call(0, method, params);
        response
            .get("result")
            .cloned()
            .unwrap_or_else(|| panic!("{method} failed: {line}"))
    }
}

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
                .replace(&format!("server {VERSION};"), "server $VERSION;");
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
    events.result("hello", &json!({"product": "shed", "version": VERSION}));
    let (request, response, _) = events.call(22, "events", &json!({"since": 0}));
    let replay: Vec<String> = (0..4).map(|_| events.next()).collect();
    let response = format!("{response}\n{}", replay.join("\n"));
    goldens.record("events", &(request, response, Value::Null));
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

    goldens.record(
        "hello",
        &c.call(1, "hello", &json!({"product": "shed", "version": VERSION})),
    );
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

    let other =
        json!({"name": "other", "labels": {"role": "rev"}, "cwd": dir, "cmd": ["sleep", "100"]});
    c.result("run", &other);
    let ls = c.call(3, "ls", &json!({"labels": {"role": "impl"}}));
    goldens.record("ls", &ls);
    assert_eq!(ls.2["result"].as_array().map(Vec::len), Some(1));

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
fn a_kept_terminal_comes_back_as_a_shell_in_its_cwd() {
    let dir = temp_dir("restart");
    let work = dir.join("work");
    std::fs::create_dir(&work).expect("work dir");
    let paths = Paths::in_dir(&dir);
    let server = Server::start(&dir);
    let mut client = Client::connect(&paths.socket).expect("connect");
    let run = |client: &mut Client, name: &str, keep: bool| -> TerminalInfo {
        let params = json!({
            "name": name, "labels": {"issue": "7"}, "cwd": work, "keep": keep,
            "cmd": ["sh", "-c", format!("echo out-{name}; echo {name} >> ran; exec sleep 100")],
        });
        client.call(Method::Run, &params).expect("run")
    };
    let kept = run(&mut client, "kept", true);
    run(&mut client, "lost", false);
    let ran = || std::fs::read_to_string(work.join("ran")).unwrap_or_default();
    eventually("both tasks ran", || ran().lines().count() == 2);
    run(&mut client, "killed", true);
    eventually("the third task ran", || ran().lines().count() == 3);
    let killed: TerminalInfo = client
        .call(Method::Kill, &json!({"name": "killed"}))
        .expect("kill");
    assert_eq!(killed.state, State::Exited);
    let state_mode = std::fs::metadata(&paths.state)
        .expect("state")
        .permissions()
        .mode();
    assert_eq!(state_mode & 0o777, 0o600);
    let read = |client: &mut Client, name: &str| -> String {
        let read = ReadParams {
            name: name.into(),
            lines: None,
            history: true,
            ..read_params()
        };
        client
            .call::<_, ReadResult>(Method::Read, &read)
            .expect("read")
            .text
    };
    for name in ["kept", "lost"] {
        eventually("the output", || {
            read(&mut client, name) == format!("out-{name}")
        });
    }
    drop(client);
    drop(server);

    let _server = Server::start(&dir);
    let mut client = Client::connect(&paths.socket).expect("reconnect");
    let back: TerminalInfo = client
        .call(Method::Get, &json!({"name": "kept"}))
        .expect("get");
    assert_eq!(back.state, State::Running);
    assert_eq!(back.cmd, ["/bin/sh"]);
    assert_eq!((back.cwd, back.labels), (work.clone(), kept.labels));
    assert_ne!(back.pid, kept.pid);
    eventually("the kept history", || {
        read(&mut client, "kept").starts_with("out-kept\n")
    });
    assert_eq!(
        read(&mut client, "lost"),
        "out-lost",
        "a lost terminal reads its log"
    );
    let lost: TerminalInfo = client
        .call(Method::Get, &json!({"name": "lost"}))
        .expect("get");
    assert_eq!(
        (lost.state, lost.exit, lost.pid),
        (State::Exited, None, None)
    );

    let still: TerminalInfo = client
        .call(Method::Get, &json!({"name": "killed"}))
        .expect("get");
    assert_eq!(
        still, killed,
        "a kept terminal that had exited stays exited"
    );

    let send = SendParams {
        name: "kept".into(),
        text: "echo \"in:$PWD\"\n".into(),
    };
    client.call::<_, Value>(Method::Send, &send).expect("send");
    let want = format!("in:{}", work.display());
    eventually("the shell's pwd", || {
        let read = ReadParams {
            name: "kept".into(),
            lines: None,
            history: false,
            ..read_params()
        };
        let screen: ReadResult = client.call(Method::Read, &read).expect("read");
        screen.text.replace('\n', "").contains(&want)
    });
    assert_eq!(
        ran(),
        "kept\nlost\nkilled\n",
        "a kept task must never re-run"
    );
}

#[test]
fn a_version_mismatch_is_refused() {
    let dir = temp_dir("version");
    let _server = Server::start(&dir);
    let mut c = Conn::open(&dir);
    let (_, line, _) = c.call(
        1,
        "hello",
        &json!({"product": "shed", "version": "0.0.0-other"}),
    );
    assert_eq!(
        line,
        format!(
            r#"{{"jsonrpc":"2.0","id":1,"error":{{"code":-32000,"message":"SHED_VERSION: client shed 0.0.0-other, server shed {VERSION}; restart the server"}}}}"#
        )
    );
    let (_, _, after) = c.call(2, "ls", &json!({}));
    assert_eq!(
        after["error"]["code"], -32001,
        "a refused client stays refused"
    );

    let mut old = Conn::open(&dir);
    let (_, line, _) = old.call(1, "hello", &json!({"version": VERSION}));
    assert_eq!(
        line,
        format!(
            r#"{{"jsonrpc":"2.0","id":1,"error":{{"code":-32000,"message":"SHED_VERSION: client unnamed {VERSION}, server shed {VERSION}; restart the server"}}}}"#
        )
    );
}

#[test]
fn events_stream_in_seq_order_filter_by_label_and_replay_since() {
    let dir = temp_dir("events");
    let _server = Server::start(&dir);
    let mut c = Conn::open(&dir);
    c.result("hello", &json!({"product": "shed", "version": VERSION}));
    let mut live = Conn::events(&dir, &json!({"labels": {"test": "order"}}));

    let script = "printf '\\033]7501;state=working\\007'; read x; exit 3";
    let run = |name: &str, labels: Value, cmd: Value| json!({"name": name, "labels": labels, "cwd": dir, "cmd": cmd});
    c.result("run", &run("other", json!({}), json!(["sleep", "100"])));
    c.result(
        "run",
        &run("a", json!({"test": "order"}), json!(["sh", "-c", script])),
    );
    let started = live.event();
    assert_eq!(
        (&started["event"], &started["name"]),
        (&json!("started"), &json!("a"))
    );
    assert_eq!(started["labels"], json!({"test": "order"}));
    let state = live.event();
    assert_eq!(
        (&state["event"], &state["state"]),
        (&json!("state"), &json!("working"))
    );
    c.result("send", &json!({"name": "a", "text": "\n"}));
    let exited = live.event();
    assert_eq!(
        (&exited["event"], &exited["code"]),
        (&json!("exited"), &json!(3))
    );

    c.result(
        "run",
        &run("b", json!({"test": "order"}), json!(["sleep", "100"])),
    );
    assert_eq!(live.event()["event"], "started");
    c.result("kill", &json!({"name": "b"}));
    let killed = live.event();
    assert_eq!(
        (&killed["event"], &killed["name"]),
        (&json!("killed"), &json!("b"))
    );

    assert_eq!(started["seq"], 2);
    assert_eq!(state["seq"], 3);
    assert_eq!(exited["seq"], 4);
    assert_eq!(killed["seq"], 6);

    let mut all = Conn::events(&dir, &json!({"since": 0}));
    let seqs: Vec<Value> = (0..6).map(|_| all.event()["seq"].clone()).collect();
    assert_eq!(seqs, [1, 2, 3, 4, 5, 6].map(|n| json!(n)));
    let mut since = Conn::events(&dir, &json!({"since": 3, "labels": {"test": "order"}}));
    let replay: Vec<Value> = (0..3).map(|_| since.event()["seq"].clone()).collect();
    assert_eq!(replay, [4, 5, 6].map(|n| json!(n)));

    c.result(
        "run",
        &run("c", json!({"test": "order"}), json!(["sleep", "100"])),
    );
    for stream in [&mut since, &mut live, &mut all] {
        assert_eq!(
            stream.event()["seq"],
            7,
            "after the replay, the live stream"
        );
    }
}

#[test]
fn jan_osc_9999_states_are_reported_with_their_source_and_garbled_ones_ignored() {
    let dir = temp_dir("jan");
    let _server = Server::start(&dir);
    let mut c = Conn::open(&dir);
    c.result("hello", &json!({"product": "shed", "version": VERSION}));
    let mut live = Conn::events(&dir, &json!({}));
    let jan = |state: &str| format!(r#"\033]9999;{{"agentType":"jan","state":"{state}"}}\007"#);
    let script = format!(
        "printf 'a{}b'; read x; printf '{}{}{}{}c'; read x",
        jan("working"),
        r#"\033]9999;{"agentType":"jan","sta\007"#,
        r"\033]9999;not json\007",
        jan("sleeping"),
        jan("done"),
    );
    let params = json!({"name": "jan", "cwd": dir, "cmd": ["sh", "-c", script]});
    c.result("run", &params);
    assert_eq!(live.event()["event"], "started");
    let state = |e: Value| {
        (
            e["state"].clone(),
            e["state_source"].clone(),
            e["event"].clone(),
        )
    };
    let working = (json!("working"), json!("jan-osc9999"), json!("state"));
    assert_eq!(state(live.event()), working);
    c.result("send", &json!({"name": "jan", "text": "\n"}));
    let done = (json!("done"), json!("jan-osc9999"), json!("state"));
    assert_eq!(state(live.event()), done);
    let info = c.result("get", &json!({"name": "jan"}));
    assert_eq!(
        info["reported"],
        json!({"state": "done", "state_source": "jan-osc9999"})
    );
    let read = c.result("read", &json!({"name": "jan"}));
    assert_eq!(read["text"], "ab\nc", "no escape reaches the screen");
}

fn hello(dir: &Path) -> Conn {
    let mut c = Conn::open(dir);
    c.result("hello", &json!({"product": "shed", "version": VERSION}));
    c
}

fn shell(c: &mut Conn, dir: &Path, name: &str) {
    c.result("run", &json!({"name": name, "cwd": dir, "cmd": ["sh"]}));
    eventually("the prompt", || {
        c.result("read", &json!({"name": name}))["text"] == "$"
    });
}

fn rows(update: &Value) -> Vec<(u64, String)> {
    let lines = update["lines"].as_array().expect("lines");
    lines
        .iter()
        .map(|l| {
            let runs = l["runs"].as_array().expect("runs");
            let text = runs.iter().map(|r| r["text"].as_str().expect("text"));
            (l["row"].as_u64().expect("row"), text.collect())
        })
        .collect()
}

fn until_row(watch: &mut Conn, text: &str) -> Vec<Value> {
    let mut updates = Vec::new();
    loop {
        let update = watch.event();
        let done = rows(&update).iter().any(|(_, t)| t == text);
        updates.push(update);
        if done {
            return updates;
        }
    }
}

#[test]
fn screen_watch_sends_the_full_screen_then_diffs_and_since_skips_the_full_one() {
    let dir = temp_dir("watch");
    let _server = Server::start(&dir);
    let mut c = hello(&dir);
    shell(&mut c, &dir, "w");

    let mut watch = hello(&dir);
    assert_eq!(
        watch.result("screen.watch", &json!({"terminal": "w"})),
        json!({})
    );
    let first = watch.event();
    assert_eq!(first["full"], true);
    assert_eq!((&first["rows"], &first["cols"]), (&json!(24), &json!(80)));
    assert_eq!(first["cursor"], json!([0, 2]));
    let all = rows(&first);
    assert_eq!(all.len(), 24);
    assert_eq!(all[0], (0, "$".to_owned()));

    c.result(
        "terminal.input",
        &json!({"terminal": "w", "text": "printf '\\033[1;31mred\\033[0m\\n'\n"}),
    );
    let updates = until_row(&mut watch, "red");
    let last = updates.last().expect("an update");
    assert!(updates.iter().all(|u| u["full"] == false), "{updates:?}");
    assert!(rows(last).len() < 24, "only changed rows: {last}");
    let red = &last["lines"]
        .as_array()
        .expect("lines")
        .iter()
        .find(|l| l["row"] == 1)
        .expect("row 1")["runs"][0];
    assert_eq!(
        red,
        &json!({"text": "red", "fg": 1, "bg": null, "bold": true, "italic": false, "underline": false, "inverse": false})
    );

    eventually("the prompt after the output", || {
        c.result("read", &json!({"name": "w"}))["text"]
            .as_str()
            .is_some_and(|t| t.ends_with("red\n$"))
    });
    let mut again = hello(&dir);
    again.result("screen.watch", &json!({"terminal": "w"}));
    let seq = again.event()["seq"].clone();
    let mut since = hello(&dir);
    since.result("screen.watch", &json!({"terminal": "w", "since": seq}));
    c.result("terminal.input", &json!({"terminal": "w", "text": "x"}));
    let next = since.event();
    assert_eq!(next["full"], false, "{next}");
    assert_eq!(rows(&next), [(2, "$ x".to_owned())]);
}

#[test]
fn screen_watch_sends_at_most_20_updates_a_second() {
    let dir = temp_dir("throttle");
    let _server = Server::start(&dir);
    let mut c = hello(&dir);
    shell(&mut c, &dir, "w");
    let mut watch = hello(&dir);
    watch.result("screen.watch", &json!({"terminal": "w"}));
    watch.event();

    let start = std::time::Instant::now();
    let flood = "i=0; while [ $i -lt 3000 ]; do echo l$i; i=$((i+1)); done; echo end\n";
    c.result("terminal.input", &json!({"terminal": "w", "text": flood}));
    let n = until_row(&mut watch, "end").len();
    let elapsed = start.elapsed().as_millis();
    assert!(
        (u128::try_from(n).unwrap() - 1) * 50 <= elapsed,
        "{n} updates in {elapsed} ms"
    );
}

#[test]
fn terminal_input_and_keys_arrive_and_a_readonly_client_is_refused() {
    let dir = temp_dir("input");
    let _server = Server::start(&dir);
    let mut c = hello(&dir);
    shell(&mut c, &dir, "w");
    c.result(
        "terminal.input",
        &json!({"terminal": "w", "text": "echo typed"}),
    );
    c.result(
        "terminal.keys",
        &json!({"terminal": "w", "keys": ["Esc", "Tab", "Left", "Right", "Enter"]}),
    );
    eventually("the echo", || {
        c.result("read", &json!({"name": "w"}))["text"]
            .as_str()
            .is_some_and(|t| t.contains("\ntyped\n"))
    });
    let (_, _, bad) = c.call(
        1,
        "terminal.keys",
        &json!({"terminal": "w", "keys": ["hyper"]}),
    );
    assert_eq!(bad["error"]["code"], -32602);

    let mut viewer = hello(&dir);
    assert_eq!(
        viewer.result("terminal.readonly", &json!({"terminal": "w"})),
        json!({})
    );
    let refused = [
        (
            "terminal.input",
            json!({"terminal": "w", "text": "echo nope\n"}),
        ),
        (
            "terminal.keys",
            json!({"terminal": "w", "keys": ["ctrl-c"]}),
        ),
        (
            "terminal.resize",
            json!({"terminal": "w", "cols": 10, "rows": 5}),
        ),
        ("send", json!({"name": "w", "text": "echo nope\n"})),
        ("keys", json!({"name": "w", "keys": ["enter"]})),
        ("resize", json!({"name": "w", "cols": 10, "rows": 5})),
        (
            "resize",
            json!({"name": "w", "terminal": "other", "cols": 10, "rows": 5}),
        ),
        ("attach", json!({"name": "w"})),
        (
            "terminal.resize",
            json!({"name": "w", "cols": 10, "rows": 5}),
        ),
    ];
    for (method, params) in refused {
        let (_, line, _) = viewer.call(1, method, &params);
        assert!(
            line.contains(r#""code":-32006,"message":"READONLY: w""#),
            "{method}: {line}"
        );
    }
    let (_, _, missing) = viewer.call(1, "terminal.readonly", &json!({"terminal": "nope"}));
    assert_eq!(missing["error"]["code"], -32002);

    c.result(
        "terminal.input",
        &json!({"terminal": "w", "text": "echo done\n"}),
    );
    eventually("the owner's echo", || {
        c.result("read", &json!({"name": "w"}))["text"]
            .as_str()
            .is_some_and(|t| t.contains("\ndone\n"))
    });
    let text = c.result("read", &json!({"name": "w"}))["text"].clone();
    assert!(!text.as_str().is_some_and(|t| t.contains("nope")), "{text}");
    let size = c.result("get", &json!({"name": "w"}));
    assert_eq!(size["state"], "running");
}

#[test]
fn terminal_resize_from_the_watcher_resizes_the_screen_and_the_program() {
    let dir = temp_dir("resize");
    let _server = Server::start(&dir);
    let mut c = hello(&dir);
    shell(&mut c, &dir, "w");
    let mut watch = hello(&dir);
    watch.result("screen.watch", &json!({"terminal": "w"}));
    assert_eq!(watch.event()["cols"], 80);

    let resize = json!({"jsonrpc": "2.0", "id": 9, "method": "terminal.resize", "params": {"cols": 40, "rows": 10}});
    watch
        .writer
        .write_all(format!("{resize}\n").as_bytes())
        .expect("write");
    let resized = watch.event();
    assert_eq!(
        (&resized["full"], &resized["rows"], &resized["cols"]),
        (&json!(true), &json!(10), &json!(40)),
        "{resized}"
    );
    assert_eq!(rows(&resized).len(), 10);

    c.result(
        "terminal.input",
        &json!({"terminal": "w", "text": "stty size\n"}),
    );
    until_row(&mut watch, "10 40");
    let (_, _, none) = c.call(1, "terminal.resize", &json!({"cols": 40, "rows": 10}));
    assert_eq!(none["error"]["code"], -32602);
}

fn read_params() -> ReadParams {
    serde_json::from_value(serde_json::json!({"name": ""})).expect("read params")
}
