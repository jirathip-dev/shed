use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::Output;

use crate::common::{Server, eventually, temp_dir};
use crate::{SHED, shed_in, stdout};

fn zero_pids(s: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(at) = rest.find("\"pid\":") {
        let (head, tail) = rest.split_at(at + 6);
        out.push_str(head);
        let spaces = tail.len() - tail.trim_start_matches(' ').len();
        let (gap, tail) = tail.split_at(spaces);
        out.push_str(gap);
        let digits = tail.len() - tail.trim_start_matches(|c: char| c.is_ascii_digit()).len();
        out.push_str(if digits > 0 { "0" } else { "" });
        rest = &tail[digits..];
    }
    out + rest
}

struct Transcript {
    dir: PathBuf,
    text: String,
    golden: &'static str,
}

impl Transcript {
    fn run(&mut self, args: &[&str]) -> Output {
        let out = shed_in(&self.dir, args);
        let dir = self.dir.to_str().expect("utf-8 dir");
        let mut text = format!(
            "$ shed {}\n{}",
            args.join(" "),
            String::from_utf8_lossy(&out.stdout)
        );
        if !out.status.success() {
            let code = out.status.code().unwrap_or(-1);
            let err = String::from_utf8_lossy(&out.stderr);
            let err = if err.is_empty() { "\n".into() } else { err };
            write!(text, "[exit {code}] {err}").expect("write to a String");
        }
        let text = text.replace(SHED, "shed").replace(dir, "$DIR");
        self.text.push_str(&zero_pids(&text));
        out
    }

    fn check(&self) {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/golden")
            .join(self.golden);
        if std::env::var_os("SHED_BLESS").is_some() {
            std::fs::write(&path, &self.text).expect("write golden");
        }
        let have = std::fs::read_to_string(&path).unwrap_or_default();
        assert_eq!(
            have,
            self.text,
            "{} differs; SHED_BLESS=1 to update",
            path.display()
        );
    }
}

#[test]
fn run_read_send_exit_ls_and_labels_match_the_golden() {
    let dir = temp_dir("cli");
    let d = dir.to_str().expect("utf-8 dir");
    let mut t = Transcript {
        dir: dir.clone(),
        text: String::new(),
        golden: "cli.txt",
    };
    t.run(&["ls"]);
    t.run(&["run", "--cwd", d, "--", "true"]);
    let _server = Server::start(&dir);

    let run = [
        "run",
        "--name",
        "shell",
        "--label",
        "role=impl",
        "--label",
        "issue=7",
    ];
    t.run(&[&run[..], &["--cwd", d, "--", "sh"]].concat());
    eventually("the prompt", || {
        stdout(&shed_in(&dir, &["read", "shell"])) == "$\n"
    });
    t.run(&["read", "shell"]);
    t.run(&["--json", "read", "shell", "--lines", "1"]);
    t.run(&["send", "shell", "exit"]);
    eventually("the shell to exit", || {
        stdout(&shed_in(&dir, &["get", "shell"])).contains("\"exited\"")
    });
    t.run(&["ls"]);

    let rev = ["run", "--name", "rev", "--label", "role=rev", "--cwd", d];
    t.run(&[&["--json"], &rev[..], &["--", "sleep", "100"]].concat());
    t.run(&["kill", "rev"]);
    t.run(&["ls", "--label", "role=impl"]);
    t.run(&["ls", "--label", "role=impl", "--label", "issue=8"]);
    t.run(&["--json", "ls", "--label", "role=rev"]);
    t.run(&["get", "shell"]);
    t.run(&["--json", "get", "shell"]);
    t.run(&["top"]);
    t.run(&["rm", "shell"]);
    t.run(&["--json", "rm", "rev"]);
    t.run(&["ls"]);
    t.run(&["keys", "rev", "enter"]);
    t.run(&["run", "--label", "nolabel", "--", "true"]);
    t.check();
}

#[test]
fn a_fake_agent_reports_by_escape_and_shed_state_and_wait_returns_on_each() {
    let dir = temp_dir("state");
    let d = dir.to_str().expect("utf-8 dir");
    let _server = Server::start(&dir);
    let mut t = Transcript {
        dir: dir.clone(),
        text: String::new(),
        golden: "state.txt",
    };
    let agent = format!(
        "printf 'start\\033]7501;state=working;note=step 1\\007end\\n'; read x; \
         '{SHED}' --dir '{d}' state done --note 'all green'; read y"
    );
    t.run(&[
        "run", "--name", "agent", "--cwd", d, "--", "sh", "-c", &agent,
    ]);
    t.run(&[
        "wait",
        "agent",
        "--until",
        "done",
        "working",
        "--timeout",
        "10",
    ]);
    t.run(&["read", "agent"]);
    t.run(&["read", "agent", "--history"]);
    t.run(&["send", "agent", ""]);
    t.run(&[
        "--json",
        "wait",
        "agent",
        "--until",
        "done",
        "--timeout",
        "10",
    ]);
    t.run(&["wait", "agent", "--until", "blocked", "--timeout", "0.2"]);
    t.run(&["state", "agent", "blocked", "--note", "needs review"]);
    t.run(&["--json", "get", "agent"]);
    t.run(&["state", "agent", "sleeping"]);
    t.run(&["send", "agent", ""]);
    t.run(&["wait", "agent", "--until", "working"]);
    t.check();
}

#[test]
fn program_status_osc_7501_keys_ids_probe_clear_and_lifetime() {
    let dir = temp_dir("osc7501");
    let d = dir.to_str().expect("utf-8 dir");
    let _server = Server::start(&dir);
    let mut t = Transcript {
        dir: dir.clone(),
        text: String::new(),
        golden: "program-status.txt",
    };
    let osc = |body: &str| format!("\\033]7501;{body}\\007");
    let agent = [
        "stty raw -echo; printf '\\033]7501;?\\007'; r=$(dd bs=1 count=85 2>/dev/null)".to_owned(),
        "printf 'probe %s\\r\\n' \"$r\" | tr '\\033\\007' 'EB'".to_owned(),
        format!(
            "printf '{}{}{}'",
            osc("state=working:id=build"),
            osc("state=done:id=build/tests"),
            osc("state=blocked:id=lint:kind=permission")
        ),
        format!(
            "printf '\\033]7501;state=working:progress=40:app=make:title=RG9jcw==:msg=QnVpbGRpbmcgZG9jcw==\\033\\\\'; read x; printf '{}{}'",
            osc("state=clear:id=build"),
            osc("state=blocked:kind=question:msg=T3ZlcndyaXRlPw==:note=x:zz")
        ),
        format!("read x; printf '{}cleared\\r\\n'; read x", osc("state=clear")),
    ]
    .join("; ");
    t.run(&[
        "run", "--name", "agent", "--cwd", d, "--", "sh", "-c", &agent,
    ]);
    t.run(&["wait", "agent", "--until", "working", "--timeout", "10"]);
    t.run(&["read", "agent"]);
    t.run(&["--json", "get", "agent"]);
    t.run(&["send", "agent", "\n"]);
    let blocked = ["wait", "agent", "--until", "blocked", "--kind"];
    t.run(&[&blocked[..], &["auth", "--timeout", "0.2"]].concat());
    t.run(&[&blocked[..], &["question", "--timeout", "10"]].concat());
    t.run(&["--json", "get", "agent"]);
    t.run(&["send", "agent", "\n"]);
    t.run(&["wait", "agent", "--output", "cleared", "--timeout", "10"]);
    t.run(&["--json", "get", "agent"]);
    for (name, report) in [("w", "state=working"), ("d", "state=done")] {
        let script = format!(
            "printf '{}{}{}ready\\n'; read x",
            osc(report),
            osc("state=blocked:id=asks"),
            osc("state=error:id=tests")
        );
        t.run(&["run", "--name", name, "--cwd", d, "--", "sh", "-c", &script]);
        t.run(&["wait", name, "--output", "ready", "--timeout", "10"]);
        t.run(&["send", name, "\n"]);
        t.run(&["wait", name, "--output", "never", "--timeout", "10"]);
        t.run(&["--json", "get", name]);
    }
    t.check();
}
