use std::io::Write as _;
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Duration;

use serde_json::{Value, json};
use shed_server::Paths;
use shed_server::proto::{Hello, Method, Request};

use crate::common::{self, temp_dir};
use crate::{SHED, box_ssh, linked, shed_on};

fn allow_all(dir: &Path) {
    let guard = "import json, sys\nfor l in sys.stdin:\n    print(json.dumps({'id': json.loads(l)['id'], 'allow': True}), flush=True)";
    let cmd = json!(["python3", "-u", "-c", guard]);
    std::fs::write(
        Paths::in_dir(dir).config,
        format!("[script]\nguard = {cmd}\n"),
    )
    .unwrap();
}

fn lines(params: &Value) -> Vec<u8> {
    let hello = Request::new(
        1,
        Method::Hello,
        serde_json::to_value(Hello::ours()).unwrap(),
    );
    let script = Request::new(2, Method::Script, params.clone());
    let mut bytes = Vec::new();
    for request in [hello, script] {
        bytes.extend(serde_json::to_vec(&request).unwrap());
        bytes.push(b'\n');
    }
    bytes
}

fn sleeping(dir: &Path) -> (Value, Box<dyn Fn() -> bool>) {
    let (pid, marker) = (dir.join("pid"), dir.join("marker"));
    std::fs::remove_file(&pid).ok();
    let sh = format!("sleep 30 & echo $! > {pid:?}; wait; touch {marker:?}");
    let code = format!("await shed.exec({{ cmd: ['sh', '-c', {sh:?}] }}); return 1;");
    let started = move || std::fs::read_to_string(&pid).is_ok_and(|p| p.ends_with('\n'));
    (json!({ "code": code }), Box::new(started))
}

fn ended(dir: &Path) {
    let pid = std::fs::read_to_string(dir.join("pid")).unwrap();
    common::eventually("the remote exec's group to end", || {
        let alive = Command::new("kill")
            .args(["-0", pid.trim()])
            .stderr(Stdio::null())
            .status();
        !alive.unwrap().success()
    });
    shed_server::clock::sleep(Duration::from_millis(300));
    assert!(!dir.join("marker").exists());
}

#[test]
fn a_script_over_a_link_has_its_own_connection_and_ends_with_its_run() {
    let root = temp_dir("link-script");
    let (_remote, _local) = linked(&root);
    let remote = root.join("remote");
    allow_all(&remote);
    let before = box_ssh(&root);
    let out = shed_on(&root, &["--on", "box", "script", "-", "--json"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .and_then(|mut child| {
            let code = "return (await shed.exec({ cmd: ['echo', 'on box'] })).code;";
            child.stdin.take().unwrap().write_all(code.as_bytes())?;
            child.wait_with_output()
        })
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    let reply: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(
        (&reply["result"], &reply["acting"][0]["method"]),
        (&json!(0), &json!("exec"))
    );
    assert_eq!(box_ssh(&root) - before, 1, "a connection of its own");

    let (mut params, started) = sleeping(&remote);
    params["machine"] = json!("box");
    let mut local = UnixStream::connect(Paths::in_dir(&root.join("local")).socket).unwrap();
    local.write_all(&lines(&params)).unwrap();
    common::eventually("the exec on box to start", &*started);
    drop(local);
    ended(&remote);
}

#[test]
fn shed_connect_ends_when_its_reader_is_gone() {
    let dir = temp_dir("connect-reader");
    let _server = common::Server::start(&dir);
    allow_all(&dir);
    let (params, started) = sleeping(&dir);
    let mut connect = Command::new(SHED)
        .arg("--dir")
        .arg(&dir)
        .arg("connect")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = connect.stdin.take().unwrap();
    stdin.write_all(&lines(&params)).unwrap();
    common::eventually("the exec to start", &*started);
    drop(connect.stdout.take());
    ended(&dir);
    connect.wait().unwrap();
    drop(stdin);
}
