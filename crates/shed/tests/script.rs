//! The `script` method end to end, against a real server: the guard, the acting list, the deadline and
//! cancelling, and `speed_report` for targets 2 and 6 (`just script-bench`).
#![cfg(test)]

mod common;

use std::io::{BufRead as _, BufReader, Write as _};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

use common::{Server, eventually, temp_dir};
use serde_json::{Value, json};
use shed_client::Conn;
use shed_server::Paths;
use shed_server::proto::{Hello, Method, Request};

const SHED: &str = env!("CARGO_BIN_EXE_shed");

const GUARD: &str = r#"
import json, os, sys
mode, dir = sys.argv[1], sys.argv[2]
with open(os.path.join(dir, "starts"), "a") as f:
    f.write(f"{os.getpid()}\n")
for line in sys.stdin:
    asked = json.loads(line)
    with open(os.path.join(dir, "asked"), "a") as f:
        f.write(line)
    if mode == "exit":
        sys.exit(3)
    if mode == "hang":
        sys.stdin.read()
    role = (asked.get("caller") or {}).get("role")
    allow = mode == "allow" or (mode == "deny-rev" and role != "rev")
    answer = {"id": asked["id"] + (mode == "wrong-id"), "allow": allow, "reason": "lanes are stopped with drover kill"}
    print(json.dumps(answer), flush=True)
"#;

fn guard(dir: &Path, mode: &str, per: &str) {
    std::fs::write(dir.join("guard.py"), GUARD).unwrap();
    let py = dir.join("guard.py");
    let cmd = json!(["python3", "-u", py, mode, dir]);
    let wait = if mode == "hang" { 1 } else { 30 };
    let config =
        format!("[script]\nguard = {cmd}\nguard_timeout = {wait}\nguard_per = \"{per}\"\n");
    std::fs::write(Paths::in_dir(dir).config, config).unwrap();
}

fn hook(dir: &Path, sh: &str) {
    let cmd = json!(["sh", "-c", sh]);
    let wait = if sh.starts_with("sleep") { 1 } else { 30 };
    let config = format!("[script]\nguard = {cmd}\nguard_timeout = {wait}\nguard_per = \"call\"\n");
    std::fs::write(Paths::in_dir(dir).config, config).unwrap();
}

fn script(dir: &Path, params: &Value) -> Value {
    let mut conn = Conn::connect(&Paths::in_dir(dir).socket).unwrap();
    conn.call(Method::Script, params).unwrap()
}

fn code(dir: &Path, code: &str) -> Value {
    script(dir, &json!({ "code": code }))
}

fn error_code(reply: &Value) -> &str {
    reply["error"]["code"]
        .as_str()
        .unwrap_or_else(|| panic!("no error: {reply}"))
}

fn lines(dir: &Path, name: &str) -> Vec<String> {
    let text = std::fs::read_to_string(dir.join(name)).unwrap_or_default();
    text.lines().map(str::to_owned).collect()
}

fn alive(pid: &str) -> bool {
    Command::new("kill")
        .args(["-0", pid])
        .status()
        .unwrap()
        .success()
}

fn start(dir: &Path, name: &str) {
    let d = dir.to_str().unwrap();
    let out = Command::new(SHED)
        .args([
            "--dir",
            d,
            "run",
            "--name",
            name,
            "--cwd",
            d,
            "--label",
            "fleet=perf",
        ])
        .args(["--size", "24x80", "--", "sh", "-c"])
        .arg("printf 'line one\\nline two\\nDo you want to proceed?\\n'; exec cat >/dev/null")
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
}

fn touch(marker: &Path) -> String {
    format!(
        "await shed.exec({{ cmd: [\"touch\", {:?}] }}); return 1;",
        marker.to_str().unwrap()
    )
}

#[test]
fn no_guard_refuses_what_acts_and_a_read_needs_no_guard() {
    let dir = temp_dir("script-noguard");
    let _server = Server::start(&dir);
    let marker = dir.join("marker");
    let reply = code(&dir, &touch(&marker));
    assert_eq!(error_code(&reply), "NO_GUARD", "{reply}");
    assert!(!marker.exists());
    assert_eq!(reply["acting"][0]["error"]["code"], "NO_GUARD");
    start(&dir, "t");
    let read = "const l = await shed.ls({}); const r = await shed.read({ name: l[0].name }); return r.text.includes('line two');";
    let mut reply = code(&dir, read);
    reply.as_object_mut().unwrap().remove("ms");
    assert_eq!(
        reply,
        json!({"result": true, "logs": [], "calls": 2, "acting": []})
    );
}

#[test]
fn each_guard_failure_refuses_per_run_and_per_call() {
    let dir = temp_dir("script-guards");
    let _server = Server::start(&dir);
    let marker = dir.join("marker");
    let cases = [
        ("deny", "run", "GUARD_REFUSED"),
        ("exit", "run", "GUARD_FAILED"),
        ("wrong-id", "run", "GUARD_FAILED"),
        ("hang", "run", "GUARD_TIMEOUT"),
        ("allow", "call", "GUARD_FAILED"),
    ];
    for (mode, per, want) in cases {
        guard(&dir, mode, per);
        let reply = code(&dir, &touch(&marker));
        assert_eq!(error_code(&reply), want, "{mode} per {per}: {reply}");
        assert_eq!(reply["error"]["name"], "GuardError");
        assert!(!marker.exists(), "{mode} per {per} ran the exec");
    }
    hook(&dir, "sleep 5");
    assert_eq!(error_code(&code(&dir, &touch(&marker))), "GUARD_TIMEOUT");
    hook(
        &dir,
        "cat >/dev/null; echo first >&2; echo no pushing here >&2; exit 3",
    );
    let reply = code(&dir, &touch(&marker));
    assert_eq!(error_code(&reply), "GUARD_REFUSED");
    assert_eq!(
        reply["error"]["message"],
        "exec refused by the guard: no pushing here"
    );
    let config = Paths::in_dir(&dir).config;
    std::fs::write(&config, "[script]\nguard = [\"/no/such/guard\"]\n").unwrap();
    assert_eq!(error_code(&code(&dir, &touch(&marker))), "GUARD_FAILED");
    assert!(!marker.exists());
    hook(&dir, "cat >/dev/null");
    assert_eq!(code(&dir, &touch(&marker))["result"], 1);
    assert!(marker.exists());
}

#[test]
fn one_guard_process_serves_a_run_and_ends_with_it() {
    let dir = temp_dir("script-one");
    let _server = Server::start(&dir);
    start(&dir, "t");
    guard(&dir, "allow", "run");
    let reads = "for (let i = 0; i < 5; i++) { await shed.get({ name: 't' }); } return 0;";
    assert_eq!(code(&dir, reads)["result"], 0);
    assert!(
        lines(&dir, "starts").is_empty(),
        "a run of reads started a guard"
    );
    let acts = "for (let i = 0; i < 50; i++) { await shed.state({ name: 't', state: 'working' }); } return 0;";
    let reply = code(&dir, acts);
    assert_eq!(reply["acting"].as_array().unwrap().len(), 50, "{reply}");
    let starts = lines(&dir, "starts");
    assert_eq!(starts.len(), 1, "{starts:?}");
    assert_eq!(lines(&dir, "asked").len(), 50);
    eventually("the guard to end with its run", || !alive(&starts[0]));
}

#[test]
fn the_guard_gets_the_role_and_a_role_never_allows_more() {
    let dir = temp_dir("script-role");
    let _server = Server::start(&dir);
    start(&dir, "t");
    guard(&dir, "deny-rev", "run");
    let kill = "await shed.kill({ name: 't' }); return 'killed';";
    let caller = |role| json!({ "terminal": "impl-190", "role": role });
    let reply = script(&dir, &json!({ "code": kill, "caller": caller("rev") }));
    assert_eq!(error_code(&reply), "GUARD_REFUSED");
    assert_eq!(
        reply["error"]["message"],
        "kill refused by the guard: lanes are stopped with drover kill"
    );
    let asked: Value = serde_json::from_str(&lines(&dir, "asked")[0]).unwrap();
    assert_eq!(asked["caller"], caller("rev"));
    assert_eq!(asked["params"], json!({ "name": "t" }));
    std::fs::remove_file(Paths::in_dir(&dir).config).unwrap();
    let reply = script(
        &dir,
        &json!({ "code": kill, "caller": caller("orchestrator") }),
    );
    assert_eq!(error_code(&reply), "NO_GUARD");
}

#[test]
fn the_reply_lists_every_acting_call() {
    let dir = temp_dir("script-acting");
    let _server = Server::start(&dir);
    guard(&dir, "allow", "run");
    let d = dir.to_str().unwrap();
    let start = format!(
        "const lane = await shed.run({{ name: 'impl-192', cwd: {d:?}, cmd: ['cat'] }});
         await shed.get({{ name: lane.name }});
         await shed.send({{ name: lane.name, text: 'hi' }});
         return lane.name;"
    );
    let reply = code(&dir, &start);
    assert_eq!(reply["result"], "impl-192", "{reply}");
    let acting = reply["acting"].as_array().unwrap();
    let methods: Vec<&str> = acting
        .iter()
        .map(|a| a["method"].as_str().unwrap())
        .collect();
    assert_eq!(methods, ["run", "send"]);
    assert_eq!(acting[0]["result"]["name"], "impl-192");
    assert_eq!(
        acting[1],
        json!({"method": "send", "params": {"name": "impl-192", "text": "hi"}, "result": {}})
    );
    assert_eq!(reply["calls"], 3);
    let late = format!(
        "console.log('before'); await shed.exec({{ cmd: ['sleep', '30'], cwd: {d:?} }}); return 1;"
    );
    let reply = script(&dir, &json!({ "code": late, "timeout": 5 }));
    assert_eq!(error_code(&reply), "DEADLINE");
    assert_eq!(reply["logs"], json!(["before"]));
    assert_eq!(reply["acting"][0]["error"]["code"], "CANCELLED");
}

#[test]
fn acting_over_the_limit_keeps_each_method() {
    let dir = temp_dir("script-limit");
    let _server = Server::start(&dir);
    guard(&dir, "allow", "run");
    start(&dir, "t");
    let writes = "const note = 'x'.repeat(300000);
         for (let i = 0; i < 20; i++) { await shed.state({ name: 't', state: 'working', note }); }
         return 1;";
    let reply = code(&dir, writes);
    assert_eq!(error_code(&reply), "REPLY_LIMIT", "{}", reply["error"]);
    let acting = reply["acting"].as_array().unwrap();
    assert_eq!(acting.len(), 20);
    assert!(
        acting
            .iter()
            .all(|a| *a == json!({"method": "state", "result": true}))
    );
    let big = code(&dir, "return 'x'.repeat(9000000);");
    assert_eq!(error_code(&big), "REPLY_LIMIT");
    assert!(big.get("result").is_none());
}

#[test]
fn a_hung_wait_stops_at_the_deadline() {
    let dir = temp_dir("script-deadline");
    let _server = Server::start(&dir);
    start(&dir, "t");
    let wait = "await shed.wait({ name: 't', until: ['done'] }); return 1;";
    let started = Instant::now();
    let reply = script(&dir, &json!({ "code": wait, "timeout": 1 }));
    assert_eq!(error_code(&reply), "DEADLINE");
    assert!(
        reply["error"]["message"]
            .as_str()
            .unwrap()
            .ends_with("in flight: wait"),
        "{reply}"
    );
    assert!(started.elapsed() < Duration::from_secs(15));
    let over = Conn::connect(&Paths::in_dir(&dir).socket)
        .unwrap()
        .call::<_, Value>(
            Method::Script,
            &json!({ "code": "return 1", "timeout": 601 }),
        );
    assert!(over.unwrap_err().to_string().contains("INVALID_PARAMS"));
}

fn raw(dir: &Path, script: &Value) -> UnixStream {
    let mut stream = UnixStream::connect(Paths::in_dir(dir).socket).unwrap();
    let hello = Request::new(
        1,
        Method::Hello,
        serde_json::to_value(Hello::ours()).unwrap(),
    );
    let script = Request::new(2, Method::Script, script.clone());
    for request in [hello, script] {
        let mut line = serde_json::to_vec(&request).unwrap();
        line.push(b'\n');
        stream.write_all(&line).unwrap();
    }
    stream
}

#[test]
fn closing_the_connection_cancels_the_calls_in_flight() {
    let dir = temp_dir("script-cancel");
    let _server = Server::start(&dir);
    guard(&dir, "allow", "run");
    let (pid, marker) = (dir.join("pid"), dir.join("marker"));
    let sh = format!("sleep 30 & echo $! > {pid:?}; wait; touch {marker:?}");
    let code = format!("await shed.exec({{ cmd: ['sh', '-c', {sh:?}] }}); return 1;");
    let stream = raw(&dir, &json!({ "code": code }));
    eventually("the exec to start", || lines(&dir, "pid").len() == 1);
    let sleeping = lines(&dir, "pid").remove(0);
    drop(stream);
    let closed = Instant::now();
    eventually("the exec's group to end", || !alive(&sleeping));
    assert!(
        closed.elapsed() < Duration::from_secs(10),
        "{:?}",
        closed.elapsed()
    );
    std::thread::sleep(Duration::from_millis(300));
    assert!(!marker.exists());
}

#[test]
fn a_peer_that_only_stops_writing_gets_its_reply() {
    let dir = temp_dir("script-halfclose");
    let _server = Server::start(&dir);
    start(&dir, "t");
    let wait = "await shed.wait({ name: 't', until: ['done'], timeout: 1.5 }); return 'replied';";
    let stream = raw(&dir, &json!({ "code": wait }));
    stream.shutdown(std::net::Shutdown::Write).unwrap();
    let replies: Vec<Value> = BufReader::new(stream)
        .lines()
        .map(|l| serde_json::from_str(&l.unwrap()).unwrap())
        .collect();
    assert_eq!(replies.len(), 2, "{replies:?}");
    assert_eq!(replies[1]["result"]["result"], "replied");
}

#[test]
fn check_parses_without_running() {
    let dir = temp_dir("script-check");
    let _server = Server::start(&dir);
    let run = "await shed.run({ name: 'x', cmd: ['true'] }); return 1;";
    let reply = script(&dir, &json!({ "code": run, "check": true }));
    assert_eq!(reply["result"], Value::Null);
    assert_eq!(
        (reply["calls"].clone(), reply["acting"].clone()),
        (json!(0), json!([]))
    );
    assert_eq!(
        code(&dir, "return (await shed.ls({})).length;")["result"],
        0
    );
    let bad = script(
        &dir,
        &json!({ "code": "const a = 1;\nreturn /x/;", "check": true }),
    );
    assert_eq!(
        (bad["error"]["name"].clone(), bad["error"]["line"].clone()),
        (json!("SyntaxError"), json!(2))
    );
}

#[test]
fn methods_and_describe_come_from_the_protocol() {
    let dir = temp_dir("script-methods");
    let _server = Server::start(&dir);
    let reply = code(
        &dir,
        "const m = await shed.methods(); const d = await shed.describe('read');
         return [m.find((x) => x.name === 'kill').script, m.find((x) => x.name === 'events').script, d.method.name, Object.keys(d.schemas).includes('ReadResult')];",
    );
    assert_eq!(
        reply["result"],
        json!(["guarded", "refused", "read", true]),
        "{reply}"
    );
    let refused = code(&dir, "await shed.events({}); return 1;");
    assert_eq!(error_code(&refused), "NOT_IN_SCRIPT");
}

#[test]
fn every_method_has_the_class_the_readme_gives_it() {
    let readme =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/architecture/script/README.md");
    let text = std::fs::read_to_string(readme).unwrap();
    let mut listed = Vec::new();
    for class in ["read", "guarded", "refused"] {
        let row = text
            .lines()
            .find(|l| l.starts_with(&format!("| {class} | `")))
            .unwrap();
        let cells: Vec<&str> = row.split(" | ").collect();
        let names = cells[1].split('`').skip(1).step_by(2);
        listed.extend(names.map(|n| (n.to_owned(), class)));
    }
    listed.sort();
    let mut classes: Vec<(String, &str)> = Method::ALL
        .map(|m| (m.name().to_owned(), m.in_script().name()))
        .into();
    classes.sort();
    assert_eq!(listed, classes);
}

fn median(mut ms: Vec<f64>) -> (f64, f64, f64) {
    ms.sort_by(f64::total_cmp);
    (ms[ms.len() / 2], ms[0], ms[ms.len() - 1])
}

fn row(what: &str, target: &str, (median, low, high): (f64, f64, f64)) {
    println!("{what:<58} {median:>9.3}   target {target:<22} range {low:.3} to {high:.3}");
}

fn timed(f: impl FnOnce()) -> f64 {
    let start = Instant::now();
    f();
    start.elapsed().as_secs_f64() * 1000.0
}

fn shed_ok(dir: &Path, args: &[&str]) {
    let out = Command::new(SHED)
        .arg("--dir")
        .arg(dir)
        .args(args)
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
}

const BLOCKED: &str = "const lanes = await shed.ls({ labels: { fleet: 'perf' } });
const blocked = lanes.filter((t) => t.reported?.state === 'blocked');
return Promise.all(blocked.map(async (t) => {
  const { text } = await shed.read({ name: t.name, lines: 15 });
  return { name: t.name, note: t.reported.note ?? null, tail: text.split('\\n') };
}));";

#[test]
#[ignore = "a report, not a check: just script-bench"]
fn speed_report() {
    let dir = temp_dir("script-speed");
    let _server = Server::start(&dir);
    let names: Vec<String> = (0..8).map(|i| format!("lane{i}")).collect();
    for name in &names {
        start(&dir, name);
        shed_ok(
            &dir,
            &["state", name, "blocked", "--note", "Bash: cargo test"],
        );
    }
    let reads: Vec<Vec<&str>> = names
        .iter()
        .map(|n| vec!["read", n, "--lines", "15"])
        .collect();
    let per_call = (0..RUNS).map(|_| {
        timed(|| {
            shed_ok(&dir, &["ls", "--label", "fleet=perf", "--json"]);
            for r in &reads {
                shed_ok(&dir, r);
            }
        })
    });
    let per_call = median(per_call.collect());
    row("ls and 8 reads as 9 shed commands, ms", "", per_call);
    let ls = median(
        (0..RUNS)
            .map(|_| timed(|| shed_ok(&dir, &["ls", "--label", "fleet=perf", "--json"])))
            .collect(),
    );
    row("one shed ls, ms", "", ls);
    let one = median(
        (0..RUNS)
            .map(|_| timed(|| _ = code(&dir, BLOCKED)))
            .collect(),
    );
    row(
        "one script request for the same (connect, hello, script), ms",
        "at most 1.5 x one ls",
        one,
    );
    guard_rows(&dir);
    read_rows(&dir);
}

const RUNS: usize = 5;

fn guard_rows(dir: &Path) {
    let gets = "for (let i = 0; i < 2000; i++) { await shed.get({ name: 'lane0' }); } return 0;";
    let states = "for (let i = 0; i < 2000; i++) { await shed.state({ name: 'lane0', state: 'blocked' }); } return 0;";
    guard(dir, "allow", "run");
    let unguarded = median((0..RUNS).map(|_| timed(|| _ = code(dir, gets))).collect());
    let guarded = median(
        (0..RUNS)
            .map(|_| timed(|| assert_eq!(code(dir, states)["result"], 0)))
            .collect(),
    );
    row(
        "target 2: 2,000 state calls through one guard, the run, ms",
        "under 2,000",
        guarded,
    );
    row(
        "target 2: 2,000 get calls, no guard, the run, ms",
        "",
        unguarded,
    );
    let added = (guarded.0 - unguarded.0) / 2000.0;
    println!(
        "{:<58} {added:>9.3}   target under 1 ms",
        "target 2: added a call, at most (state with guard - get), ms"
    );
    hook(
        dir,
        "exec python3 -c 'import json, sys; json.loads(sys.stdin.read())'",
    );
    let few = "for (let i = 0; i < 200; i++) { await shed.state({ name: 'lane0', state: 'blocked' }); } return 0;";
    let per_call_guard = median(
        (0..RUNS)
            .map(|_| timed(|| assert_eq!(code(dir, few)["result"], 0)) / 200.0)
            .collect(),
    );
    row(
        "target 2: a process a call (200 state calls), a call, ms",
        "",
        per_call_guard,
    );
    std::fs::remove_file(Paths::in_dir(dir).config).unwrap();
}

fn read_rows(dir: &Path) {
    let read_p99 = |busy: bool| {
        let spinning: Vec<UnixStream> = (0..if busy { 8 } else { 0 })
            .map(|_| raw(dir, &json!({ "code": "while (true) {}" })))
            .collect();
        let mut conn = Conn::connect(&Paths::in_dir(dir).socket).unwrap();
        std::thread::sleep(Duration::from_millis(200));
        let mut ms: Vec<f64> = (0..2000)
            .map(|_| {
                timed(|| {
                    _ = conn
                        .call::<_, Value>(Method::Read, &json!({ "name": "lane0" }))
                        .unwrap();
                })
            })
            .collect();
        for stream in spinning {
            let fuel = BufReader::new(stream).lines().nth(1).unwrap().unwrap();
            assert!(fuel.contains("\"FUEL\""), "{fuel}");
        }
        ms.sort_by(f64::total_cmp);
        ms[ms.len() * 99 / 100]
    };
    let idle = median((0..RUNS).map(|_| read_p99(false)).collect());
    let busy = median((0..RUNS).map(|_| read_p99(true)).collect());
    row("target 6: another client's read, p99, idle, ms", "", idle);
    row(
        "target 6: another client's read, p99, 8 scripts spinning, ms",
        "at most 3",
        busy,
    );
}

fn cli(dir: &Path, args: &[&str], stdin: &str) -> std::process::Output {
    let mut child = Command::new(SHED)
        .arg("--dir")
        .arg(dir)
        .args(args)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(stdin.as_bytes())
        .unwrap();
    child.wait_with_output().unwrap()
}

fn skill(name: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../skill/script/examples");
    path.join(name).to_str().unwrap().to_owned()
}

#[test]
fn check_reports_a_syntax_error_and_makes_no_call() {
    let dir = temp_dir("script-cli-check");
    let _server = Server::start(&dir);
    guard(&dir, "allow", "run");
    let checked = cli(&dir, &["script", "--check", "-f", &skill("start.js")], "");
    let stderr = String::from_utf8_lossy(&checked.stderr);
    assert!(checked.status.success(), "{checked:?}");
    assert!(
        checked.stdout.is_empty() && stderr.starts_with("[0 calls, "),
        "{checked:?}"
    );
    let bad = cli(
        &dir,
        &["script", "--check", "-"],
        "const a = 1;\nreturn /x/;",
    );
    assert_eq!(bad.status.code(), Some(1), "{bad:?}");
    assert_eq!(
        String::from_utf8_lossy(&bad.stderr),
        "error: SyntaxError 2:8: a regex literal is not in the subset: use text.includes(…) or text.matches(pattern)\n"
    );
    assert_eq!(
        lines(&dir, "starts"),
        Vec::<String>::new(),
        "no guard was started"
    );
    assert_eq!(
        code(&dir, "return (await shed.ls({})).length;")["result"],
        0
    );
}

#[test]
fn the_skill_examples_are_the_goldens_and_run_on_a_real_server() {
    for name in ["blocked.js", "start.js", "errors.js"] {
        let golden = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../shed-script/tests/golden")
            .join(name);
        let golden = std::fs::read_to_string(golden).unwrap();
        let example = std::fs::read_to_string(skill(name)).unwrap();
        assert_eq!(golden.split_once('\n').unwrap().1, example, "{name}");
    }
    let dir = temp_dir("script-examples");
    let _server = Server::start(&dir);
    let d = dir.to_str().unwrap();
    let out = "printf 'Compiling a\\nerror[E0308]: mismatched types\\ntest result: FAILED. 1 passed; 1 failed\\n'; exec cat >/dev/null";
    shed_ok(
        &dir,
        &[
            "run",
            "--name",
            "impl-190",
            "--cwd",
            d,
            "--label",
            "fleet=shed",
            "--",
            "sh",
            "-c",
            out,
        ],
    );
    shed_ok(
        &dir,
        &["state", "impl-190", "blocked", "--note", "Bash: cargo test"],
    );
    eventually("impl-190's output", || {
        code(&dir, "return (await shed.read({ name: 'impl-190' })).text;")["result"]
            .as_str()
            .is_some_and(|t| t.contains("test result"))
    });
    let run = |name: &str| {
        let out = cli(&dir, &["script", "-f", &skill(name), "--json"], "");
        let reply: Value = serde_json::from_slice(&out.stdout).unwrap();
        (out.status.code(), reply)
    };
    let (status, blocked) = run("blocked.js");
    assert_eq!(status, Some(0), "{blocked}");
    let lane = &blocked["result"][0];
    assert_eq!(
        (&lane["name"], &lane["note"]),
        (&json!("impl-190"), &json!("Bash: cargo test"))
    );
    assert!(
        lane["tail"]
            .as_array()
            .unwrap()
            .contains(&json!("error[E0308]: mismatched types"))
    );
    let (status, errors) = run("errors.js");
    assert_eq!(status, Some(0), "{errors}");
    let kept: Vec<&Value> = errors["result"]
        .as_array()
        .unwrap()
        .iter()
        .map(|k| &k["line"])
        .collect();
    assert_eq!(
        kept,
        [
            "error[E0308]: mismatched types",
            "test result: FAILED. 1 passed; 1 failed"
        ]
    );
    assert!(
        errors["logs"][0]
            .as_str()
            .unwrap()
            .ends_with(" lines read, 2 kept"),
        "{errors}"
    );
    let (status, start) = run("start.js");
    assert_eq!(status, Some(1), "{start}");
    assert_eq!(
        (error_code(&start), &start["acting"][0]["method"]),
        ("NO_GUARD", &json!("run"))
    );
}
