//! The client against fake servers: the handshake and its refusals, the pool, `once`, the typed calls
//! and the streams.
#![cfg(test)]

use std::io::{BufRead as _, BufReader, Write as _};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Barrier, Mutex};
use std::time::Duration;

use serde_json::{Value, json};
use shed_client::proto::{
    BUILD, ErrorCode, Event, EventKind, EventsParams, Hello, Method, OLDEST_PROTOCOL, PROTOCOL,
    Request, RpcError,
};
use shed_client::{Calls as _, Conn, Error, Shed, Streamed, once};

type Answer = dyn Fn(&str, &Value, &mut UnixStream) -> Value + Send + Sync;

struct Fake {
    dir: PathBuf,
    socket: PathBuf,
    seen: Arc<Mutex<Vec<(String, Value)>>>,
    accepted: Arc<AtomicUsize>,
}

impl Drop for Fake {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.dir).ok();
    }
}

fn fake(
    name: &str,
    answer: impl Fn(&str, &Value, &mut UnixStream) -> Value + Send + Sync + 'static,
) -> Fake {
    let dir = std::env::temp_dir().join(format!("shed-client-{name}-{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    std::fs::create_dir_all(&dir).expect("temp dir");
    let socket = dir.join("shed.sock");
    let listener = UnixListener::bind(&socket).expect("bind");
    let (seen, accepted) = (Arc::default(), Arc::new(AtomicUsize::new(0)));
    let answer: Arc<Answer> = Arc::new(answer);
    let (log, count) = (Arc::clone(&seen), Arc::clone(&accepted));
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            count.fetch_add(1, Ordering::SeqCst);
            let (log, answer) = (Arc::clone(&log), Arc::clone(&answer));
            std::thread::spawn(move || serve(stream.expect("accept"), &log, &*answer));
        }
    });
    Fake {
        dir,
        socket,
        seen,
        accepted,
    }
}

fn serve(mut stream: UnixStream, log: &Mutex<Vec<(String, Value)>>, answer: &Answer) {
    let mut reader = BufReader::new(stream.try_clone().expect("clone"));
    let mut line = String::new();
    while reader.read_line(&mut line).unwrap_or(0) > 0 {
        let req: Request = serde_json::from_str(&line).expect("a request");
        line.clear();
        if req.method != "hello" {
            log.lock()
                .unwrap()
                .push((req.method.clone(), req.params.clone()));
        }
        let mut reply = answer(&req.method, &req.params, &mut stream);
        reply["jsonrpc"] = "2.0".into();
        reply["id"] = req.id.unwrap_or_default();
        if writeln!(stream, "{reply}").is_err() {
            return;
        }
    }
}

fn hello(params: &Value) -> Value {
    json!({"result": Hello::answer(&serde_json::from_value(params.clone()).unwrap())})
}

fn hello_or(method: &str, params: &Value, result: &Value) -> Value {
    if method == "hello" {
        hello(params)
    } else {
        json!({"result": result})
    }
}

fn refused_with(theirs: Value) -> String {
    let fake = fake("refused", move |_, _, _| json!({"result": theirs}));
    Conn::connect(&fake.socket)
        .err()
        .expect("refused")
        .to_string()
}

fn ours() -> String {
    format!("SHED_VERSION: this client speaks shed protocol {OLDEST_PROTOCOL} to {PROTOCOL}")
}

const FIX: &str =
    "(shed updates itself from its own releases: `shed self-update`, docs/self-update.md)";

#[test]
fn a_server_that_is_not_shed_or_shares_no_protocol_is_refused_with_its_fix() {
    let none = refused_with(json!({"version": "9.9.9"}));
    assert_eq!(none, format!("{}; unnamed unknown is not shed", ours()));
    let old = json!({"product": "shed", "version": "0.0.1", "build": "abc1234", "protocol": 0});
    assert_eq!(
        refused_with(old),
        format!(
            "{}; the server is 0.0.1 (abc1234) protocol 0 to 0; update shed there {FIX}",
            ours()
        )
    );
    let new = PROTOCOL + 1;
    let newer = json!({"product": "shed", "version": "9.0.0", "build": "def5678", "protocol": new, "oldest": new});
    assert_eq!(
        refused_with(newer),
        format!(
            "{}; the server is 9.0.0 (def5678) protocol {new} to {new}; update this client {FIX}",
            ours()
        )
    );
}

#[test]
fn a_refused_hello_is_a_version_error_with_the_servers_reason() {
    let fake = fake(
        "refusal",
        |_, _, _| json!({"error": RpcError::new(ErrorCode::Version, "client x / server y; update the client")}),
    );
    let err = Shed::at(&fake.socket).hello().expect_err("refused");
    assert!(
        matches!(
            err,
            Error::Version {
                theirs: None,
                refusal: Some(_),
                ..
            }
        ),
        "{err:?}"
    );
    assert_eq!(
        err.to_string(),
        format!(
            "{}; the server refused it (client x / server y; update the client); update the side it names {FIX}",
            ours()
        )
    );
}

#[test]
fn nothing_listening_is_an_io_error_naming_the_socket() {
    let socket = std::env::temp_dir().join(format!("shed-client-none-{}.sock", std::process::id()));
    let err = Shed::at(&socket).get("t1").expect_err("no server");
    assert!(
        matches!(&err, Error::Io { socket: s, .. } if *s == socket),
        "{err:?}"
    );
    assert!(
        err.to_string()
            .starts_with(&format!("{}: ", socket.display())),
        "{err}"
    );
}

#[test]
fn a_server_error_carries_its_code_and_close_takes_a_terminal_already_gone() {
    let fake = fake("rpc", |method, params, _| match method {
        "hello" => hello(params),
        _ => json!({"error": RpcError::new(ErrorCode::NoSuchTerminal, "t9")}),
    });
    let shed = Shed::at(&fake.socket);
    let err = shed.get("t9").expect_err("no such terminal");
    assert_eq!(err.code(), Some(ErrorCode::NoSuchTerminal));
    assert_eq!(err.to_string(), "NO_SUCH_TERMINAL: t9");
    shed.close("t9").expect("gone is closed");
    let seen = fake.seen.lock().unwrap().clone();
    assert_eq!(
        seen,
        [
            ("get".into(), json!({"name": "t9"})),
            ("kill".into(), json!({"name": "t9"}))
        ]
    );
}

#[test]
fn calls_reuse_one_connection_and_a_dropped_one_is_dialled_again_without_an_error() {
    let fake = fake("pool", |method, params, stream| {
        if method == "kill" {
            stream.shutdown(std::net::Shutdown::Both).ok();
        }
        hello_or(method, params, &json!([]))
    });
    let shed = Shed::at(&fake.socket);
    for _ in 0..3 {
        assert_eq!(shed.agents().expect("agents"), []);
    }
    assert_eq!(fake.accepted.load(Ordering::SeqCst), 1);
    shed.kill("t1").expect_err("the server hung up mid-call");
    assert_eq!(shed.agents().expect("after a hang-up"), []);
    assert_eq!(fake.accepted.load(Ordering::SeqCst), 2);
}

#[test]
fn the_pool_keeps_at_most_four_idle_connections() {
    let barrier = Arc::new(Barrier::new(6));
    let held = Arc::clone(&barrier);
    let fake = fake("four", move |method, params, _| {
        if method == "agents" {
            held.wait();
        }
        hello_or(method, params, &json!([]))
    });
    let shed = Shed::at(&fake.socket);
    let burst = || {
        let calls: Vec<_> = (0..6)
            .map(|_| {
                let shed = shed.clone();
                std::thread::spawn(move || shed.agents().expect("agents"))
            })
            .collect();
        for call in calls {
            assert_eq!(call.join().expect("call"), []);
        }
    };
    burst();
    assert_eq!(fake.accepted.load(Ordering::SeqCst), 6);
    burst();
    assert_eq!(fake.accepted.load(Ordering::SeqCst), 8);
}

#[test]
fn a_prompt_reports_working_then_sends_the_framed_text() {
    let lane = json!({"name": "lane", "labels": {}, "cwd": "/", "cmd": [], "keep": false,
        "state": "running", "exit": null, "pid": 7});
    let fake = fake("prompt", move |method, params, _| match method {
        "state" => json!({"result": lane}),
        _ => hello_or(method, params, &json!({})),
    });
    Shed::at(&fake.socket)
        .prompt("lane", "hi")
        .expect("prompted");
    let seen = fake.seen.lock().unwrap().clone();
    let text = "\u{1e}{\"prompt\":\"hi\"}\n";
    assert_eq!(
        seen,
        [
            ("state".into(), json!({"name": "lane", "state": "working"})),
            ("send".into(), json!({"name": "lane", "text": text})),
        ]
    );
}

#[test]
fn once_shakes_hands_and_calls_in_one_exchange() {
    let mut sent = String::new();
    let agents: Vec<Value> = once(Method::Agents, &json!({}), |lines| {
        sent = lines.to_owned();
        let hello = json!({"jsonrpc": "2.0", "id": 1, "result": Hello::ours()});
        Ok(format!(
            "{hello}\n{{\"jsonrpc\":\"2.0\",\"id\":2,\"result\":[]}}\n"
        ))
    })
    .expect("one exchange");
    assert_eq!(agents, Vec::<Value>::new());
    let methods: Vec<String> = sent
        .lines()
        .map(|l| serde_json::from_str::<Request>(l).expect("request").method)
        .collect();
    assert_eq!(methods, ["hello", "agents"]);
    assert_eq!(Hello::ours().build, BUILD);
}

#[test]
fn a_stream_gives_items_gaps_heartbeats_and_lines_it_cannot_read() {
    let event = json!({"seq": 3, "name": "t1", "labels": {}, "event": "started"});
    let fake = fake("events", move |method, params, stream| {
        if method == "hello" {
            return hello(params);
        }
        let lines = [
            r#"{"jsonrpc":"2.0","id":2,"result":{}}"#.to_owned(),
            event.to_string(),
            r#"{"jsonrpc":"2.0","method":"heartbeat"}"#.to_owned(),
            r#"{"seq":4,"name":"t1","labels":{},"event":"no-such-event"}"#.to_owned(),
            r#"{"jsonrpc":"2.0","method":"reset","params":{"seq":7}}"#.to_owned(),
            r#"{"jsonrpc":"2.0","method":"lagged","params":{"seq":9}}"#.to_owned(),
        ];
        writeln!(stream, "{}", lines.join("\n")).ok();
        stream.shutdown(std::net::Shutdown::Both).ok();
        json!({})
    });
    let mut events = Shed::at(&fake.socket)
        .events(&EventsParams::default())
        .expect("subscribed");
    let first = events.next();
    assert!(
        matches!(
            &first,
            Some(Streamed::Item(Event {
                seq: 3,
                kind: EventKind::Started,
                ..
            }))
        ),
        "{first:?}"
    );
    assert_eq!(events.since(), Some(3));
    assert_eq!(events.next(), Some(Streamed::Quiet));
    let skipped = r#"{"seq":4,"name":"t1","labels":{},"event":"no-such-event"}"#;
    assert_eq!(events.next(), Some(Streamed::Skipped(skipped.into())));
    assert_eq!(events.since(), Some(3));
    assert_eq!(events.next(), Some(Streamed::Reset(7)));
    assert_eq!(events.since(), Some(7));
    assert_eq!(events.next(), Some(Streamed::Lagged(9)));
    assert_eq!(events.next(), None);
}

#[test]
fn a_ticking_stream_says_quiet_while_the_server_is_silent() {
    let fake = fake("tick", |method, params, _| {
        hello_or(method, params, &json!({}))
    });
    let mut screen = Shed::at(&fake.socket)
        .watch("t1", None)
        .expect("watching")
        .tick(Duration::from_millis(1));
    let quiet = screen.next();
    assert!(matches!(quiet, Some(Streamed::Quiet)), "{quiet:?}");
}
