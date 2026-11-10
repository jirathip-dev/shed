//! `shed install|uninstall|doctor` with `HOME` in a temp dir: the job file they write and check.
#![cfg(test)]

mod common;

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use common::{Server, temp_dir};

const SHED: &str = env!("CARGO_BIN_EXE_shed");

fn shed(home: &Path, args: &[&str]) -> Output {
    Command::new(SHED)
        .env("HOME", home)
        .env_remove("XDG_CONFIG_HOME")
        .args(args)
        .output()
        .expect("shed runs")
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
    let start = if cfg!(target_os = "macos") {
        format!("launchctl bootstrap gui/$(id -u) {}", file.display())
    } else {
        "systemctl --user daemon-reload && systemctl --user enable --now shed.service".to_owned()
    };

    let out = shed(&home, &["install"]);
    assert!(out.status.success(), "{out:?}");
    assert!(
        text(&out).contains(&format!("wrote {}", file.display())),
        "{out:?}"
    );
    assert!(text(&out).contains(&start), "{out:?}");
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
    assert!(text(&out).contains(&start), "{out:?}");
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
    let out = shed(&home, &["install", "--force"]);
    assert!(out.status.success(), "{out:?}");
    assert_eq!(std::fs::read_to_string(&file).expect("job"), job);

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
    let version = format!("ok    version  shed {}\n", env!("CARGO_PKG_VERSION"));
    assert!(report.contains(&version), "{report}");
}
