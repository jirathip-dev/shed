use std::process::{Command, Output, Stdio};

use serde_json::{Value, json};

use crate::common::{Server, eventually, temp_dir};
use crate::{SHED, info, shed_in, shed_ok, start};

#[test]
fn a_child_started_inside_a_terminal_has_that_parent() {
    let dir = temp_dir("parent");
    let _server = Server::start(&dir);
    let d = dir.to_str().expect("utf-8 dir");
    let script = format!(
        "'{SHED}' --dir '{d}' run --name kid -- sleep 100; \
         '{SHED}' --dir '{d}' run --no-parent --name loner -- sleep 100; read x"
    );
    start(&dir, "lead", &[], &["sh", "-c", &script]);
    eventually("both started", || {
        shed_in(&dir, &["get", "loner"]).status.success()
    });
    assert_eq!(info(&dir, "kid")["parent"], json!({"name": "lead"}));
    assert_eq!(info(&dir, "loner").get("parent"), None);
    let kids: Vec<Value> =
        serde_json::from_str(&shed_ok(&dir, &["--json", "ls", "--parent", "lead"])).expect("ls");
    assert_eq!(kids.len(), 1);
    assert_eq!(kids[0]["name"], "kid");
    let refused = shed_in(&dir, &["run", "--parent", "nope", "--", "true"]);
    assert!(!refused.status.success());
    assert!(String::from_utf8_lossy(&refused.stderr).contains("parent nope is not running"));
}

#[test]
fn wait_children_all_returns_only_after_both_children_report_done() {
    let dir = temp_dir("children");
    let _server = Server::start(&dir);
    start(&dir, "lead", &[], &["sleep", "100"]);
    for kid in ["a", "b"] {
        start(&dir, kid, &["--parent", "lead"], &["sleep", "100"]);
    }
    let all = ["wait", "--children", "all", "lead", "--timeout"];
    let mut waiter = Command::new(SHED)
        .arg("--dir")
        .arg(&dir)
        .args([&all[..], &["20"]].concat())
        .stdout(Stdio::piped())
        .spawn()
        .expect("wait starts");
    shed_ok(&dir, &["state", "a", "done"]);
    let early = shed_in(&dir, &[&all[..], &["0.3"]].concat());
    assert_eq!(early.status.code(), Some(124), "{early:?}");
    assert!(waiter.try_wait().expect("try_wait").is_none());
    shed_ok(&dir, &["state", "b", "done"]);
    let done = waiter.wait_with_output().expect("wait ends");
    assert_eq!(done.status.code(), Some(0));
    assert_eq!(
        String::from_utf8_lossy(&done.stdout),
        "lead no child working\n"
    );
}

#[test]
fn wait_children_any_returns_on_the_next_child_with_news() {
    let dir = temp_dir("any");
    let _server = Server::start(&dir);
    start(&dir, "lead", &[], &["sleep", "100"]);
    for kid in ["a", "b"] {
        start(&dir, kid, &["--parent", "lead"], &["sleep", "100"]);
    }
    let mut waiter = Command::new(SHED)
        .arg("--dir")
        .arg(&dir)
        .args([
            "--json",
            "wait",
            "--children",
            "any",
            "lead",
            "--timeout",
            "20",
        ])
        .stdout(Stdio::piped())
        .spawn()
        .expect("wait starts");
    eventually("news of b ends the wait", || {
        shed_ok(&dir, &["state", "a", "working"]);
        shed_ok(&dir, &["state", "b", "working"]);
        shed_ok(&dir, &["state", "b", "blocked"]);
        waiter.try_wait().expect("try_wait").is_some()
    });
    let done = waiter.wait_with_output().expect("wait ends");
    assert_eq!(done.status.code(), Some(0));
    let done: Value = serde_json::from_slice(&done.stdout).expect("wait JSON");
    assert_eq!(
        (&done["outcome"], &done["terminal"]["name"]),
        (&json!("matched"), &json!("b"))
    );
    shed_ok(&dir, &["kill", "a"]);
    shed_ok(&dir, &["kill", "b"]);
    let none = shed_in(
        &dir,
        &["wait", "--children", "any", "lead", "--timeout", "10"],
    );
    assert_eq!(stdout_code(&none), ("lead exited\n".to_owned(), Some(1)));
}

fn stdout_code(out: &Output) -> (String, Option<i32>) {
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        out.status.code(),
    )
}

#[test]
fn killing_the_parent_ends_a_child_and_spares_one_started_to_outlive_it() {
    let dir = temp_dir("cascade");
    let _server = Server::start(&dir);
    start(&dir, "lead", &[], &["sleep", "100"]);
    start(&dir, "kid", &["--parent", "lead"], &["sleep", "100"]);
    start(
        &dir,
        "free",
        &["--parent", "lead", "--outlive-parent"],
        &["sleep", "100"],
    );
    shed_ok(&dir, &["kill", "lead"]);
    let ended = shed_in(&dir, &["wait", "kid", "--until", "done", "--timeout", "10"]);
    assert_eq!(
        stdout_code(&ended),
        ("kid exited 143\n".to_owned(), Some(1))
    );
    assert_eq!(info(&dir, "free")["state"], "running");

    start(&dir, "quits", &[], &["sh", "-c", "read x"]);
    start(&dir, "kid2", &["--parent", "quits"], &["sleep", "100"]);
    shed_ok(&dir, &["send", "quits", "\n"]);
    let ended = shed_in(
        &dir,
        &["wait", "kid2", "--until", "done", "--timeout", "10"],
    );
    assert_eq!(
        stdout_code(&ended),
        ("kid2 exited 143\n".to_owned(), Some(1))
    );
}

#[test]
fn notify_puts_one_note_in_an_idle_parent_and_none_while_it_works() {
    let dir = temp_dir("notes");
    let _server = Server::start(&dir);
    start(&dir, "lead", &[], &["sh", "-c", "while read l; do :; done"]);
    start(
        &dir,
        "a",
        &["--parent", "lead", "--notify"],
        &["sleep", "100"],
    );
    start(
        &dir,
        "b",
        &["--parent", "lead", "--notify"],
        &["sleep", "100"],
    );
    start(&dir, "quiet", &["--parent", "lead"], &["sleep", "100"]);
    let screen = || shed_ok(&dir, &["read", "lead"]);
    let count = |text: &str, line: &str| text.lines().filter(|l| *l == line).count();
    let upto = |mark: &str| {
        eventually(mark, || screen().contains(mark));
        let text = screen();
        text.split(mark).next().unwrap_or_default().to_owned()
    };

    shed_ok(&dir, &["state", "quiet", "blocked"]);
    shed_ok(&dir, &["state", "a", "blocked", "--note", "needs review"]);
    shed_ok(&dir, &["state", "a", "blocked", "--note", "needs review"]);
    shed_ok(&dir, &["send", "lead", "mark1\n"]);
    let before = upto("mark1");
    assert_eq!(
        count(&before, "[shed] a blocked: needs review"),
        1,
        "{before}"
    );
    assert!(!before.contains("[shed] quiet"), "{before}");

    shed_ok(&dir, &["state", "lead", "working"]);
    shed_ok(&dir, &["state", "b", "error"]);
    shed_ok(&dir, &["send", "lead", "mark2\n"]);
    let before = upto("mark2");
    assert!(!before.contains("[shed] b"), "{before}");

    shed_ok(&dir, &["state", "lead", "done"]);
    eventually("b's note once lead is done", || {
        count(&screen(), "[shed] b error") == 1
    });
    shed_ok(&dir, &["send", "lead", "mark3\n"]);
    let all = upto("mark3");
    assert_eq!(count(&all, "[shed] a blocked: needs review"), 1, "{all}");
    assert_eq!(count(&all, "[shed] b error"), 1, "{all}");
}
