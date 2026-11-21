use std::fmt::Write as _;
use std::os::unix::fs::PermissionsExt as _;
use std::path::Path;
use std::process::Command;

use serde_json::{Value, json};
use shed_server::lane::transcript::Preview;
use shed_server::proto::{EventKind, Logged, ReadResult};

use crate::common::{Server, temp_dir};
use crate::{SHED, events_until, fake_agent, is_state, ok, registry};

#[test]
fn a_turn_with_an_approval_matches_the_golden_event_log() {
    let dir = temp_dir("acp-golden");
    let _server = Server::start(&dir);
    let reg = registry(&dir);
    ok(
        &dir,
        &[
            "agents",
            "add",
            "fake",
            "--registry",
            reg.to_str().expect("utf-8"),
        ],
    );
    let mut log = String::new();
    let mut step = |args: &[&str]| {
        let out = ok(&dir, args);
        write!(log, "$ shed {}\n{out}", args.join(" ")).expect("write to a String");
    };
    step(&[
        "run",
        "--name",
        "a",
        "--label",
        "run=golden",
        "--acp",
        "fake",
    ]);
    step(&[
        "wait",
        "a",
        "--output",
        "ACP session ready",
        "--timeout",
        "10",
    ]);
    step(&["send", "a", "tool"]);
    step(&[
        "wait",
        "a",
        "--until",
        "waiting-approval",
        "--timeout",
        "10",
    ]);
    step(&["approve", "a", "allow-once"]);
    step(&["wait", "a", "--until", "done", "--timeout", "10"]);
    step(&["read", "a"]);
    step(&["send", "a", "hi"]);
    step(&["wait", "a", "--until", "done", "--timeout", "10"]);
    step(&["read", "a", "--lines", "2"]);
    let dones = std::cell::Cell::new(0);
    let events = events_until(&dir, "golden", |e| {
        dones.set(dones.get() + usize::from(is_state("done")(e)));
        dones.get() == 2
    });
    let events = events.join("\n");
    writeln!(log, "$ shed events --label run=golden --since 0\n{events}").expect("write");
    let transcript = std::fs::read_to_string(dir.join("sessions/sess-1.jsonl"));
    let transcript = transcript.expect("the transcript, by session id");
    assert_eq!(transcript.lines().count(), 14, "{transcript}");
    let record: Value = serde_json::from_slice(
        &std::fs::read(dir.join("sessions/sess-1.json")).expect("the record"),
    )
    .expect("JSON");
    assert_eq!(
        (&record["agent"], &record["title"], &record["turns"]),
        (&json!("fake"), &json!("tool"), &json!(2))
    );
    let mode = |p: &str| {
        std::fs::metadata(dir.join(p))
            .expect(p)
            .permissions()
            .mode()
            & 0o777
    };
    assert_eq!(
        [
            mode("sessions"),
            mode("sessions/sess-1.json"),
            mode("sessions/sess-1.jsonl")
        ],
        [0o700, 0o600, 0o600]
    );

    let golden = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden/acp.txt");
    if std::env::var_os("SHED_BLESS").is_some() {
        std::fs::write(&golden, &log).expect("write golden");
    }
    let have = std::fs::read_to_string(&golden).unwrap_or_default();
    assert_eq!(
        have,
        log,
        "{} differs; SHED_BLESS=1 to update",
        golden.display()
    );
}

fn golden(name: &str, have: &str) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/golden")
        .join(name);
    if std::env::var_os("SHED_BLESS").is_some() {
        std::fs::write(&path, have).expect("write golden");
    }
    let want = std::fs::read_to_string(&path).unwrap_or_default();
    assert_eq!(
        want,
        have,
        "{} differs; SHED_BLESS=1 to update",
        path.display()
    );
}

#[test]
fn recorded_sessions_of_three_agents_read_and_draw_as_the_same_blocks() {
    let dir = temp_dir("acp-replay");
    let _server = Server::start(&dir);
    let fake = fake_agent();
    let mut shapes = Vec::new();
    for agent in ["claude", "codex", "gemini"] {
        let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/acp");
        let fixture = fixtures.join(format!("{agent}.jsonl"));
        let label = format!("run={agent}");
        let run = ["run", "--name", agent, "--label", &label, "--acp", "fake"];
        let cmd = [
            "--policy",
            "allow",
            "--",
            "python3",
            fake.to_str().expect("utf-8"),
        ];
        ok(
            &dir,
            &[
                &run[..],
                &cmd,
                &["--replay", fixture.to_str().expect("utf-8")],
            ]
            .concat(),
        );
        ok(
            &dir,
            &[
                "wait",
                agent,
                "--output",
                "ACP session ready",
                "--timeout",
                "10",
            ],
        );
        ok(&dir, &["send", agent, "replay"]);
        ok(&dir, &["wait", agent, "--until", "done", "--timeout", "10"]);

        let json = ok(&dir, &["read", agent, "--events", "--json"]);
        let read: ReadResult = serde_json::from_str(&json).expect("a read result");
        golden(&format!("acp-{agent}.events.jsonl"), &json);
        let mut preview = Preview::default();
        for e in read.events.clone() {
            preview.push(e.kind);
        }
        let colour = preview.draw(80, true, usize::MAX).join("\n") + "\n";
        golden(&format!("acp-{agent}.80.txt"), &colour);
        let out = Command::new(SHED)
            .arg("--dir")
            .arg(&dir)
            .args(["read", agent, "--events"])
            .env("NO_COLOR", "1")
            .output()
            .expect("shed runs");
        assert!(out.status.success(), "{out:?}");
        let plain = String::from_utf8(out.stdout).expect("utf-8");
        assert!(!plain.contains('\x1b'), "{plain}");
        assert!(plain.lines().all(|l| l.chars().count() <= 80), "{plain}");
        golden(&format!("acp-{agent}.no-color.txt"), &plain);

        let title = (read.events.iter())
            .find_map(|e| match &e.kind {
                EventKind::ToolCall { title, kind, .. } if kind.as_deref() == Some("execute") => {
                    Some(title.clone())
                }
                _ => None,
            })
            .expect("an execute tool call");
        let (first, second) = title.split_once('\n').expect("a multi-line title");
        let line = format!("✓ execute {first}");
        let line: String = line.chars().take(80).collect();
        assert_eq!(plain.lines().filter(|l| *l == line).count(), 1, "{plain}");
        assert!(!plain.contains(second.trim()), "{plain}");
        let escape = |c: char| {
            let slash = if c.is_ascii_punctuation() { "\\" } else { "" };
            format!("{slash}{c}")
        };
        let pattern: String = line.chars().map(escape).collect();
        ok(
            &dir,
            &["wait", agent, "--output", &pattern, "--timeout", "5"],
        );

        shapes.push(shape(&read.events));
    }
    assert_eq!(shapes[0], shapes[1], "claude and codex");
    assert_eq!(shapes[0], shapes[2], "claude and gemini");
    assert_eq!(shapes[0].len(), 8, "{:?}", shapes[0]);
}

fn shape(events: &[Logged]) -> Vec<String> {
    let mut shape: Vec<String> = Vec::new();
    let mut seen = std::collections::BTreeMap::new();
    for e in events {
        let at = shape.len();
        match &e.kind {
            EventKind::ToolCall {
                id, kind, status, ..
            } => {
                let at = *seen.entry(id.clone()).or_insert(at);
                let block = format!("tool {} {status}", kind.as_deref().unwrap_or("-"));
                if at == shape.len() {
                    shape.push(block);
                } else {
                    shape[at] = block;
                }
            }
            EventKind::Text { .. } | EventKind::Mode { .. } | EventKind::Session(_) => {}
            kind => shape.push(serde_json::to_value(kind).expect("JSON")["event"].to_string()),
        }
    }
    shape
}
