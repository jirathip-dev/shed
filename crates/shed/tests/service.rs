//! `shed install|uninstall|doctor` with `HOME` in a temp dir: the job file they write and check. A fake
//! `launchctl` and `systemctl` on `PATH` log what install runs, so no test touches the real job.
#![cfg(test)]

mod common;

use std::io::{BufRead as _, BufReader, Write as _};
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use common::{Server, temp_dir};
use serde_json::json;
use shed_server::proto::{BUILD, LONG_VERSION, PROTOCOL};

const SHED: &str = env!("CARGO_BIN_EXE_shed");

fn shed(home: &Path, args: &[&str]) -> Output {
    let bin = home.join("fake-bin");
    std::fs::create_dir_all(&bin).expect("fake bin");
    for ctl in ["launchctl", "systemctl"] {
        let file = bin.join(ctl);
        let log = "echo \"$(basename \"$0\") $*\" >> \"$(dirname \"$0\")/ctl.log\"\n";
        std::fs::write(&file, format!("#!/bin/sh\n{log}")).expect("fake ctl");
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    }
    let path = std::env::var("PATH").unwrap_or_default();
    Command::new(SHED)
        .env("HOME", home)
        .env("PATH", format!("{}:{path}", bin.display()))
        .env_remove("XDG_CONFIG_HOME")
        .args(args)
        .output()
        .expect("shed runs")
}

fn ctl_log(home: &Path) -> String {
    std::fs::read_to_string(home.join("fake-bin/ctl.log")).unwrap_or_default()
}

fn text(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned() + &String::from_utf8_lossy(&out.stderr)
}

fn job_file(home: &Path) -> PathBuf {
    if cfg!(target_os = "macos") {
        home.join("Library/LaunchAgents/dev.shed.serve.plist")
    } else {
        home.join(".config/systemd/user/shed.service")
    }
}

#[test]
fn install_writes_the_job_once_and_refuses_other_contents_without_force() {
    let home = temp_dir("install");
    let file = job_file(&home);
    let uid = Command::new("id").arg("-u").output().expect("id -u").stdout;
    let uid = String::from_utf8_lossy(&uid).trim().to_owned();
    let restarted = if cfg!(target_os = "macos") {
        format!(
            "launchctl bootout gui/{uid}/dev.shed.serve\nlaunchctl bootstrap gui/{uid} {}\n",
            file.display()
        )
    } else {
        "systemctl --user disable --now shed.service\nsystemctl --user daemon-reload\n\
         systemctl --user enable --now shed.service\n"
            .to_owned()
    };

    let out = shed(&home, &["install"]);
    assert!(out.status.success(), "{out:?}");
    assert!(
        text(&out).contains(&format!("wrote {}", file.display())),
        "{out:?}"
    );
    assert!(text(&out).contains("restarted dev.shed.serve"), "{out:?}");
    assert_eq!(
        ctl_log(&home),
        restarted,
        "install stops and starts the job"
    );
    let job = std::fs::read_to_string(&file).expect("the job file");
    assert!(job.contains(&format!(
        "{}/.cargo/bin:{}/.local/bin",
        home.display(),
        home.display()
    )));
    if cfg!(target_os = "macos") {
        for want in [
            "<key>RunAtLoad</key><true/>",
            "<key>KeepAlive</key><true/>",
            "<string>serve</string>",
        ] {
            assert!(job.contains(want), "{want} in {job}");
        }
        let log = home.join("Library/Logs/shed.log");
        assert!(job.contains(&format!(
            "<key>StandardOutPath</key><string>{}</string>",
            log.display()
        )));
        if let Ok(lint) = Command::new("plutil").arg("-lint").arg(&file).output() {
            assert!(lint.status.success(), "plutil -lint: {lint:?}");
        }
    } else {
        assert!(
            job.contains(" serve\n") && job.contains("WantedBy=default.target"),
            "{job}"
        );
    }

    let before = std::fs::metadata(&file)
        .and_then(|m| m.modified())
        .expect("mtime");
    let out = shed(&home, &["install"]);
    assert!(out.status.success(), "{out:?}");
    assert!(text(&out).contains("is up to date"), "{out:?}");
    assert_eq!(
        ctl_log(&home),
        restarted.repeat(2),
        "an up-to-date install still restarts the server on this binary"
    );
    assert_eq!(
        std::fs::metadata(&file)
            .and_then(|m| m.modified())
            .expect("mtime"),
        before
    );

    std::fs::write(&file, "hand-made\n").expect("edit the job");
    let out = shed(&home, &["install"]);
    assert!(!out.status.success(), "{out:?}");
    assert!(text(&out).contains("--force"), "{out:?}");
    assert_eq!(std::fs::read_to_string(&file).expect("job"), "hand-made\n");
    assert_eq!(
        ctl_log(&home),
        restarted.repeat(2),
        "a refused install restarts nothing"
    );
    let out = shed(&home, &["install", "--force"]);
    assert!(out.status.success(), "{out:?}");
    assert_eq!(std::fs::read_to_string(&file).expect("job"), job);
    assert_eq!(ctl_log(&home), restarted.repeat(3));

    let out = shed(&home, &["uninstall"]);
    assert!(out.status.success(), "{out:?}");
    let stop = if cfg!(target_os = "macos") {
        "launchctl bootout gui/$(id -u)/dev.shed.serve"
    } else {
        "systemctl --user disable --now shed.service"
    };
    assert!(text(&out).contains(stop), "{out:?}");
    assert!(
        text(&out).contains(&format!("rm {}", file.display())),
        "{out:?}"
    );
    assert!(file.exists(), "uninstall only prints the lines");
}

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
    let version = format!("ok    version  shed {LONG_VERSION}\n");
    assert!(report.contains(&version), "{report}");
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
