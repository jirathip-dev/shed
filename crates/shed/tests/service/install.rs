use std::path::{Path, PathBuf};
use std::process::Command;

use crate::common::temp_dir;
use crate::{SHED, ctl_log, job_file, shed, text};

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
        "<key>StartInterval</key><integer>300</integer>"
    } else {
        "OnUnitActiveSec=5min"
    };
    assert!(update.contains(every), "{update}");
    assert!(
        update.contains(&format!("{SHED}</string><string>self-update<"))
            || update.contains(&format!("ExecStart={SHED} self-update\n")),
        "{update}"
    );
}

fn restarted(home: &Path) -> String {
    let file = job_file(home);
    let uid = Command::new("id").arg("-u").output().expect("id -u").stdout;
    let uid = String::from_utf8_lossy(&uid).trim().to_owned();
    let update = update_files(home);
    if cfg!(target_os = "macos") {
        format!(
            "launchctl bootout gui/{uid}/dev.shed.serve\nlaunchctl bootstrap gui/{uid} {}\n\
             launchctl bootout gui/{uid}/dev.shed.update\nlaunchctl bootstrap gui/{uid} {}\n",
            file.display(),
            update.first().expect("the update job").display()
        )
    } else {
        "systemctl --user disable --now shed.service\nsystemctl --user daemon-reload\n\
         systemctl --user enable --now shed.service\n\
         systemctl --user disable --now shed-update.timer\nsystemctl --user daemon-reload\n\
         systemctl --user enable --now shed-update.timer\n"
            .to_owned()
    }
}

fn update_files(home: &Path) -> Vec<PathBuf> {
    if cfg!(target_os = "macos") {
        vec![home.join("Library/LaunchAgents/dev.shed.update.plist")]
    } else {
        let units = home.join(".config/systemd/user");
        vec![
            units.join("shed-update.service"),
            units.join("shed-update.timer"),
        ]
    }
}

#[test]
fn install_writes_the_job_once_and_refuses_other_contents_without_force() {
    let home = temp_dir("install");
    let file = job_file(&home);
    let restarted = restarted(&home);

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

#[cfg(target_os = "linux")]
#[test]
fn install_force_over_a_unit_without_the_oom_policy_leaves_one_with_it() {
    let home = temp_dir("oom");
    let file = job_file(&home);
    std::fs::create_dir_all(file.parent().expect("unit dir")).expect("unit dir");
    std::fs::write(&file, "[Service]\nExecStart=shed serve\nRestart=always\n").expect("old unit");
    let out = shed(&home, &["install", "--force"]);
    assert!(out.status.success(), "{out:?}");
    let unit = std::fs::read_to_string(&file).expect("the unit");
    assert!(unit.contains("\nOOMPolicy=continue\n"), "{unit}");
    assert!(ctl_log(&home).contains("daemon-reload"), "reloaded");
}
