use std::path::{Path, PathBuf};
use std::process::Command;

use crate::common::temp_dir;
use crate::{SHED, ctl_log, job_file, shed, text, uid};

#[test]
fn install_writes_the_five_minute_update_job() {
    let home = temp_dir("install-update");
    assert!(shed(&home, &["install"]).status.success());
    let update = update_files(&home);
    let update = update
        .iter()
        .map(std::fs::read_to_string)
        .collect::<Result<String, _>>();
    let update = update.expect("the update job's files");
    let every = if cfg!(target_os = "macos") {
        "<key>StartInterval</key>\n  <integer>300</integer>"
    } else {
        "OnUnitActiveSec=300\n"
    };
    assert!(update.contains(every), "{update}");
    assert!(
        update.contains(&format!("{SHED}</string>\n    <string>self-update<"))
            || update.contains(&format!("ExecStart={SHED} self-update\n")),
        "{update}"
    );
}

fn loads(home: &Path) -> usize {
    let verb = if cfg!(target_os = "macos") {
        " bootstrap "
    } else {
        " enable --now "
    };
    ctl_log(home).lines().filter(|l| l.contains(verb)).count()
}

fn update_files(home: &Path) -> Vec<PathBuf> {
    if cfg!(target_os = "macos") {
        vec![home.join("Library/LaunchAgents/dev.shed.update.plist")]
    } else {
        let units = home.join(".config/systemd/user");
        vec![
            units.join("dev.shed.update.service"),
            units.join("dev.shed.update.timer"),
        ]
    }
}

#[test]
fn install_writes_and_loads_both_jobs_once_and_refuses_other_contents_without_force() {
    let home = temp_dir("install");
    let file = job_file(&home);

    let out = shed(&home, &["install"]);
    assert!(out.status.success(), "{out:?}");
    assert!(text(&out).contains("dev.shed.serve: written"), "{out:?}");
    assert!(text(&out).contains("dev.shed.update: written"), "{out:?}");
    assert_eq!(loads(&home), 2, "{}", ctl_log(&home));
    let job = std::fs::read_to_string(&file).expect("the job file");
    assert!(job.contains(&format!(
        "{}/.cargo/bin:{}/.local/bin",
        home.display(),
        home.display()
    )));
    if cfg!(target_os = "macos") {
        for want in [
            "<key>RunAtLoad</key>\n  <true/>",
            "<key>KeepAlive</key>\n  <true/>",
            "<key>ProcessType</key>\n  <string>Interactive</string>",
            "<string>serve</string>",
        ] {
            assert!(job.contains(want), "{want} in {job}");
        }
        let log = home.join("Library/Logs/dev.shed.serve.log");
        assert!(job.contains(&format!(
            "<key>StandardOutPath</key>\n  <string>{}</string>",
            log.display()
        )));
        if let Ok(lint) = Command::new("plutil").arg("-lint").arg(&file).output() {
            assert!(lint.status.success(), "plutil -lint: {lint:?}");
        }
    } else {
        for want in [
            " serve\n",
            "OOMPolicy=continue\n",
            "WantedBy=default.target",
        ] {
            assert!(job.contains(want), "{want} in {job}");
        }
    }

    let before = std::fs::metadata(&file)
        .and_then(|m| m.modified())
        .expect("mtime");
    let out = shed(&home, &["install"]);
    assert!(out.status.success(), "{out:?}");
    assert!(text(&out).contains("dev.shed.serve: unchanged"), "{out:?}");
    assert_eq!(loads(&home), 2, "an up-to-date, loaded job is left alone");
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
    assert_eq!(loads(&home), 2, "a refused install loads nothing");
    let out = shed(&home, &["install", "--force"]);
    assert!(out.status.success(), "{out:?}");
    assert_eq!(std::fs::read_to_string(&file).expect("job"), job);
    assert_eq!(loads(&home), 3, "only the rewritten job is loaded again");

    let out = shed(&home, &["uninstall"]);
    assert!(out.status.success(), "{out:?}");
    assert!(text(&out).contains("dev.shed.serve: removed"), "{out:?}");
    assert!(text(&out).contains("dev.shed.update: removed"), "{out:?}");
    let stops = if cfg!(target_os = "macos") {
        ["dev.shed.serve", "dev.shed.update"]
            .map(|l| format!("launchctl bootout gui/{}/{l}\n", uid()))
    } else {
        ["dev.shed.serve.service", "dev.shed.update.timer"]
            .map(|u| format!("systemctl --user disable --now {u}\n"))
    };
    for stop in stops {
        assert!(
            ctl_log(&home).contains(&stop),
            "{stop} in {}",
            ctl_log(&home)
        );
    }
    for gone in std::iter::once(file).chain(update_files(&home)) {
        assert!(!gone.exists(), "{} removed", gone.display());
    }
}

#[cfg(target_os = "linux")]
#[test]
fn install_replaces_the_old_unit_names_with_the_label_named_ones() {
    let home = temp_dir("oom");
    let units = home.join(".config/systemd/user");
    std::fs::create_dir_all(&units).expect("unit dir");
    let old = ["shed.service", "shed-update.service", "shed-update.timer"].map(|u| units.join(u));
    for file in &old {
        std::fs::write(file, "[Service]\nExecStart=shed serve\n").expect("old unit");
    }
    let out = shed(&home, &["install"]);
    assert!(out.status.success(), "{out:?}");
    for file in &old {
        assert!(!file.exists(), "{} removed", file.display());
    }
    let log = ctl_log(&home);
    assert!(
        log.contains("systemctl --user disable --now shed.service\n"),
        "{log}"
    );
    assert!(
        log.contains("disable --now shed-update.timer shed-update.service\n"),
        "{log}"
    );
    let unit = std::fs::read_to_string(job_file(&home)).expect("the unit");
    assert!(unit.contains("\nOOMPolicy=continue\n"), "{unit}");
}
