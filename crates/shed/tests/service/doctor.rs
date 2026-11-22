use std::io::{BufRead as _, BufReader, Write as _};
use std::path::Path;
use std::process::Command;

use serde_json::json;
use shed_server::proto::{BUILD, LONG_VERSION, PROTOCOL};

use crate::common::{Server, temp_dir};
use crate::install::old_files;
use crate::{SHED, job_file, shed, shed_with, text};

#[test]
fn doctor_reports_a_missing_job_and_server_with_their_fix_lines() {
    let home = temp_dir("doctor");
    let d = home.to_str().expect("utf-8 dir");
    let out = shed(&home, &["--dir", d, "doctor"]);
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    let report = text(&out);
    let file = job_file(&home);
    assert!(
        report.contains(&format!("FAIL  job      {}: ", file.display()))
            && report.contains("; fix: shed install\n"),
        "{report}"
    );
    let socket = home.join("shed.sock");
    assert!(
        report.contains(&format!("FAIL  socket   {}: ", socket.display())),
        "{report}"
    );

    assert!(shed(&home, &["install"]).status.success());
    let _server = Server::start(&home);
    let report = text(&shed(&home, &["--dir", d, "doctor"]));
    assert!(
        report.contains(&format!("ok    job      {}\n", file.display())),
        "{report}"
    );
    assert!(
        report.contains(&format!("ok    socket   {}\n", socket.display())),
        "{report}"
    );
    let update = "ok    update   dev.shed.update: no pass yet\n";
    assert!(report.contains(update), "{report}");
    let version = format!("ok    version  shed {LONG_VERSION}\n");
    assert!(report.contains(&version), "{report}");
    assert!(!report.contains("provider"), "{report}");

    std::fs::create_dir(home.join("providers")).expect("providers dir");
    let bad = home.join("providers/claude.toml");
    std::fs::write(&bad, "limit = \"not a list\"\n").expect("bad record");
    let out = shed(&home, &["--dir", d, "doctor"]);
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    let report = text(&out);
    let named = format!("FAIL  provider {}: ", bad.display());
    assert!(
        report.contains(&named) && report.contains("; skipped\n"),
        "{report}"
    );
    let agents: serde_json::Value =
        serde_json::from_str(&text(&shed(&home, &["--dir", d, "--json", "agents"]))).expect("JSON");
    let claude = agents
        .as_array()
        .expect("a list")
        .iter()
        .find(|a| a["id"] == "claude-acp");
    assert_eq!(
        claude.expect("claude-acp")["source"],
        "built-in",
        "the built-in record stays"
    );
}

#[test]
fn doctor_fails_when_the_server_runs_another_build() {
    let home = temp_dir("doctor-build");
    let d = home.to_str().expect("utf-8 dir");
    let listener = std::os::unix::net::UnixListener::bind(home.join("shed.sock")).expect("bind");
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept");
        let mut hello = String::new();
        BufReader::new(&stream)
            .read_line(&mut hello)
            .expect("hello");
        let other = json!({"jsonrpc": "2.0", "id": 1, "result":
            {"product": "shed", "version": "0.0.0", "build": "56de739", "protocol": PROTOCOL}});
        stream
            .write_all(format!("{other}\n").as_bytes())
            .expect("reply");
    });
    let out = shed(&home, &["--dir", d, "doctor"]);
    server.join().expect("fake server");
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    let report = text(&out);
    assert!(
        report.contains(&format!(
            "FAIL  version  binary {BUILD}, server 56de739; fix: shed install\n"
        )),
        "{report}"
    );
}

#[test]
fn setup_hooks_report_nothing_outside_shed_and_doctor_lists_them() {
    let home = temp_dir("setup");
    let d = home.to_str().expect("utf-8 dir");
    std::fs::create_dir(home.join(".claude")).expect("agent dir");
    let file = home.join(".claude/settings.json");
    let missing = format!(
        "--    hooks    claude: {} lacks them; fix: shed setup\n",
        file.display()
    );
    assert!(text(&shed(&home, &["--dir", d, "doctor"])).contains(&missing));

    let out = shed(&home, &["setup"]);
    assert!(out.status.success(), "{out:?}");
    let settings: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&file).expect("settings")).expect("json");
    let command = settings["hooks"]["Stop"][0]["hooks"][0]["command"]
        .as_str()
        .expect("hook");
    assert_eq!(command, r#"[ -z "$SHED" ] || shed state done"#);

    let bin = Path::new(SHED)
        .parent()
        .expect("bin dir")
        .display()
        .to_string();
    let path = format!("{bin}:{}", std::env::var("PATH").unwrap_or_default());
    let hook = |shed_var: Option<&str>| {
        let mut sh = Command::new("sh");
        sh.args(["-c", command])
            .env("HOME", &home)
            .env("PATH", &path)
            .env_remove("SHED");
        shed_var.map(|t| sh.env("SHED", t));
        sh.output().expect("hook runs")
    };
    let outside = hook(None);
    assert_eq!(outside.status.code(), Some(0), "{outside:?}");
    assert!(
        outside.stdout.is_empty() && outside.stderr.is_empty(),
        "{outside:?}"
    );
    assert!(
        !hook(Some("t")).status.success(),
        "inside shed it reports, here to no server"
    );

    let installed = format!("ok    hooks    claude: {}\n", file.display());
    assert!(text(&shed(&home, &["--dir", d, "doctor"])).contains(&installed));

    std::fs::write(&file, "{bad").expect("malformed settings");
    let report = text(&shed(&home, &["--dir", d, "doctor"]));
    assert!(report.contains("FAIL  hooks    "), "{report}");
    assert!(report.contains("not JSON objects"), "{report}");
}

#[test]
fn doctor_flags_a_server_outside_a_failed_job() {
    let home = temp_dir("outside");
    let d = home.to_str().expect("utf-8 dir");
    assert!(shed(&home, &["install"]).status.success());
    let _server = Server::start(&home);
    let inactive = if cfg!(target_os = "macos") {
        "print"
    } else {
        "show"
    };
    let out = shed_with(&home, &["--dir", d, "doctor"], &[("CTL_FAIL", inactive)]);
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    let line = format!(
        "FAIL  loaded   a server outside dev.shed.serve answers on {}; fix: stop that server, then ",
        home.join("shed.sock").display()
    );
    assert!(text(&out).contains(&line), "{}", text(&out));
}

#[test]
fn doctor_fails_a_leftover_old_named_update_job_until_install_removes_it() {
    let home = temp_dir("doctor-old-update");
    let d = home.to_str().expect("utf-8 dir");
    assert!(shed(&home, &["install"]).status.success());
    let old: Vec<_> = old_files(&home)
        .into_iter()
        .filter(|f| f.to_string_lossy().contains("shed-update"))
        .collect();
    for file in &old {
        std::fs::write(file, "old\n").expect("old job");
    }
    let out = shed(&home, &["--dir", d, "doctor"]);
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    let files: Vec<String> = old.iter().map(|f| f.display().to_string()).collect();
    let line = format!(
        "FAIL  update   {}: an old-named updater; fix: shed install\n",
        files.join(", ")
    );
    assert!(text(&out).contains(&line), "{line} in {}", text(&out));

    assert!(shed(&home, &["install"]).status.success());
    let report = text(&shed(&home, &["--dir", d, "doctor"]));
    let update = "ok    update   dev.shed.update: no pass yet\n";
    assert!(report.contains(update), "{report}");
}
