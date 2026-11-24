//! `run`'s and `exec`'s `sandbox`: drover's negative probes run inside a sandbox shed made, the refused writes,
//! and the inside socket's six methods.
#![cfg(test)]

mod common;

use std::io::{BufRead as _, BufReader, Write as _};
use std::os::unix::fs::PermissionsExt as _;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use common::{Server, temp_dir};
use serde_json::{Value, json};
use shed_server::proto::{ErrorCode, Hello, Method};

const SHED: &str = env!("CARGO_BIN_EXE_shed");

fn shed(dir: &Path, args: &[&str]) -> Output {
    let out = Command::new(SHED).arg("--dir").arg(dir).args(args).output();
    out.expect("shed runs")
}

fn text(out: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

fn srt() -> bool {
    let path = std::env::var_os("PATH").unwrap_or_default();
    std::env::split_paths(&path).any(|d| d.join("srt").is_file())
}

struct Lab {
    dir: PathBuf,
    home: PathBuf,
    work: PathBuf,
    caches: [PathBuf; 2],
    _server: Server,
}

fn lab(name: &str) -> Lab {
    let root = temp_dir(name);
    let (dir, home) = (root.join("d"), root.join("home"));
    let work = home.join("work");
    let caches = [
        home.join(".cache/sccache"),
        home.join(".local/share/pnpm/store"),
    ];
    for d in [&dir, &home.join(".ssh"), &work].into_iter().chain(&caches) {
        std::fs::create_dir_all(d).expect("folder");
    }
    std::fs::write(home.join(".ssh/id_probe"), "key").expect("key");
    std::fs::write(dir.join("secret-probe"), "secret").expect("secret");
    std::fs::write(home.join("other-repo"), "other").expect("other");
    let server = Server::with_env(&dir, "3600", &[("HOME", home.to_str().expect("utf-8"))]);
    Lab {
        dir,
        home,
        work,
        caches,
        _server: server,
    }
}

fn sandbox(write: &[&Path]) -> String {
    json!({"write": write, "hosts": ["github.com"]}).to_string()
}

impl Lab {
    fn exec(&self, sandbox: &str, script: &str) -> Output {
        let cwd = self.work.to_str().expect("utf-8");
        let args = [
            "exec",
            "--cwd",
            cwd,
            "--sandbox",
            sandbox,
            "--",
            "sh",
            "-c",
            script,
        ];
        shed(&self.dir, &args)
    }
}

#[test]
fn drovers_negative_probes_fail_inside_and_the_work_and_cache_folders_take_writes() {
    let lab = lab("sbx-probes");
    let [sccache, store] = &lab.caches;
    let sandbox = sandbox(&[&lab.work, sccache, store]);
    let (home, dir) = (lab.home.display(), lab.dir.display());
    let probes = [
        format!("cat {home}/.ssh/id_probe"),
        format!("ls {home}/.ssh"),
        format!("cat {dir}/secret-probe"),
        format!("cat {dir}/state.json"),
        format!("cat {home}/other-repo"),
        "curl -sS -m 10 -o /dev/null https://example.com".to_owned(),
        format!("{SHED} ls"),
    ];
    let checks = probes
        .iter()
        .map(|p| format!("{p} >/dev/null 2>&1 && echo 'OPEN {p}'"))
        .collect::<Vec<_>>()
        .join("; ");
    let script = format!(
        "echo w > w.txt && echo c > $1/c && echo s > $2/s && echo wrote; {checks}; \
         touch {home}/probe-outside; {SHED} hello >/dev/null && echo inside"
    );
    let script = format!("set -- {} {}; {script}", sccache.display(), store.display());
    let out = lab.exec(&sandbox, &script);
    if !srt() {
        assert!(text(&out).contains("SANDBOX_MISSING"), "{}", text(&out));
        assert!(!lab.work.join("w.txt").exists(), "it ran without a sandbox");
        return;
    }
    let said = text(&out);
    assert!(said.contains("wrote") && said.contains("inside"), "{said}");
    assert!(!said.contains("OPEN"), "a probe got through: {said}");
    assert!(
        lab.work.join("w.txt").is_file()
            && sccache.join("c").is_file()
            && store.join("s").is_file()
    );
    assert!(
        !lab.home.join("probe-outside").exists(),
        "it wrote outside its folders"
    );
    let rules = lab.dir.join("sandbox");
    let left: Vec<_> = std::fs::read_dir(&rules).into_iter().flatten().collect();
    assert!(left.is_empty(), "exec's rules were left behind: {left:?}");
}

#[test]
fn a_write_that_holds_sheds_own_folders_is_refused_before_anything_runs() {
    let lab = lab("sbx-refused");
    let marker = lab.work.join("ran");
    let script = format!("touch {}", marker.display());
    let root = lab.dir.parent().expect("the lab");
    for write in [root, lab.dir.as_path(), &lab.dir.join("agents")] {
        let out = lab.exec(&sandbox(&[write]), &script);
        assert!(
            text(&out).contains("INVALID_PARAMS"),
            "{write:?}: {}",
            text(&out)
        );
    }
    let bin = Path::new(SHED).parent().expect("shed's folder");
    let out = lab.exec(&sandbox(&[bin]), &script);
    assert!(text(&out).contains("INVALID_PARAMS"), "{}", text(&out));
    assert!(!marker.exists());
}

#[test]
fn a_sandboxed_run_says_so_keeps_its_rules_private_and_drops_them_with_the_terminal() {
    let lab = lab("sbx-run");
    let sandbox = sandbox(&[&lab.work]);
    let cwd = lab.work.to_str().expect("utf-8");
    let keep = [
        "run",
        "--keep",
        "--cwd",
        cwd,
        "--sandbox",
        &sandbox,
        "--",
        "sleep",
        "30",
    ];
    assert!(text(&shed(&lab.dir, &keep)).contains("INVALID_PARAMS"));
    let run = [
        "run",
        "--name",
        "s1",
        "--cwd",
        cwd,
        "--sandbox",
        &sandbox,
        "--",
        "sleep",
        "30",
    ];
    let out = shed(&lab.dir, &run);
    if !srt() {
        assert!(text(&out).contains("SANDBOX_MISSING"), "{}", text(&out));
        return;
    }
    assert!(out.status.success(), "{}", text(&out));
    let got: Value = serde_json::from_slice(&shed(&lab.dir, &["--json", "get", "s1"]).stdout)
        .expect("get's json");
    assert_eq!(got["sandboxed"], true, "{got}");
    let rules = lab.dir.join("sandbox/s1.json");
    let mode = std::fs::metadata(&rules)
        .expect("the rules")
        .permissions()
        .mode();
    assert_eq!(mode & 0o777, 0o600);
    let written: Value =
        serde_json::from_slice(&std::fs::read(&rules).expect("rules")).expect("json");
    let deny = written["filesystem"]["denyRead"].to_string();
    for own in [&lab.home, &lab.dir, &lab.home.join(".ssh")] {
        assert!(
            deny.contains(&format!("\"{}\"", own.display())),
            "{own:?} not denied: {deny}"
        );
    }
    assert!(shed(&lab.dir, &["kill", "s1"]).status.success());
    common::eventually("s1 to exit", || {
        shed(&lab.dir, &["rm", "s1"]).status.success()
    });
    assert!(!rules.exists(), "the rules outlived the terminal");
}

#[test]
fn the_inside_socket_serves_six_methods_and_no_other() {
    let dir = temp_dir("sbx-inside");
    let _server = Server::start(&dir);
    let stream = UnixStream::connect(dir.join("inside/inside.sock")).expect("inside socket");
    let mut lines = BufReader::new(stream.try_clone().expect("clone")).lines();
    let mut out = stream;
    let mut ask = |id: usize, method: Method, params: Value| -> Value {
        let line = json!({"jsonrpc": "2.0", "id": id, "method": method.name(), "params": params});
        writeln!(out, "{line}").expect("sent");
        let answer = lines.next().expect("an answer").expect("read");
        serde_json::from_str(&answer).expect("json")
    };
    let hello = ask(
        0,
        Method::Hello,
        serde_json::to_value(Hello::ours()).expect("hello"),
    );
    assert!(hello.get("result").is_some(), "{hello}");
    let served = [
        "hello",
        "heartbeat",
        "state",
        "emit",
        "slot.take",
        "slot.free",
    ];
    for (id, method) in Method::ALL.into_iter().enumerate() {
        let answer = ask(id + 1, method, json!({}));
        let code = answer["error"]["code"].as_i64();
        let not_found = code == Some(ErrorCode::MethodNotFound.code());
        assert_eq!(
            not_found,
            !served.contains(&method.name()),
            "{}: {answer}",
            method.name()
        );
    }
}

#[test]
fn an_acp_terminals_bridge_starts_its_agent_inside_the_sandbox() {
    let lab = lab("sbx-acp");
    let tests = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests");
    let rules = json!({"write": [&lab.work], "read": [&tests]}).to_string();
    let seen = lab.work.join("seen");
    let agent = format!(
        "ls {} > {} 2>&1; exec python3 {}",
        lab.home.join(".ssh").display(),
        seen.display(),
        tests.join("fake_acp.py").display()
    );
    let cwd = lab.work.to_str().expect("utf-8");
    let run = [
        "run",
        "--name",
        "a1",
        "--acp",
        "fake",
        "--cwd",
        cwd,
        "--sandbox",
        &rules,
    ];
    let out = shed(&lab.dir, &[&run[..], &["--", "sh", "-c", &agent]].concat());
    if !srt() {
        assert!(text(&out).contains("SANDBOX_MISSING"), "{}", text(&out));
        return;
    }
    assert!(out.status.success(), "{}", text(&out));
    common::eventually("the agent to start", || seen.is_file());
    let listed = std::fs::read_to_string(&seen).expect("seen");
    assert!(
        !listed.contains("id_probe"),
        "the agent ran outside: {listed}"
    );
    assert!(shed(&lab.dir, &["kill", "a1"]).status.success());
}
