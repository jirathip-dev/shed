use std::path::Path;
use std::process::{Command, Output};

use crate::common::{self, temp_dir};
use crate::{ctl_log, shed, text};

fn autostart(home: &Path) -> Output {
    let out = shed(
        home,
        &[
            "run",
            "--autostart",
            "--",
            "sh",
            "-c",
            "echo $PPID; exec sleep 100",
        ],
    );
    let mut server = String::new();
    common::eventually("the server's pid", || {
        server = String::from_utf8_lossy(&shed(home, &["read", "t1"]).stdout)
            .trim()
            .to_owned();
        !server.is_empty()
    });
    assert!(
        Command::new("kill")
            .arg(&server)
            .status()
            .expect("kill")
            .success()
    );
    common::eventually("the server to exit", || {
        !Command::new("kill")
            .args(["-0", &server])
            .stderr(std::process::Stdio::null())
            .status()
            .expect("kill -0")
            .success()
    });
    out
}

#[test]
fn autostart_starts_the_installed_job_and_no_server_of_its_own() {
    let home = temp_dir("aj");
    assert!(shed(&home, &["install"]).status.success());
    std::fs::remove_file(home.join("fake-bin/ctl.log")).expect("clear the log");
    let out = autostart(&home);
    assert!(out.status.success(), "{out:?}");
    assert!(!text(&out).contains("started"), "{out:?}");
    let kick = if cfg!(target_os = "macos") {
        "launchctl kickstart gui/"
    } else {
        "systemctl --user reset-failed shed.service\nsystemctl --user start shed.service\n"
    };
    assert!(ctl_log(&home).starts_with(kick), "{}", ctl_log(&home));
}

#[test]
fn autostart_without_a_job_starts_a_detached_server() {
    let home = temp_dir("an");
    let out = autostart(&home);
    assert!(out.status.success(), "{out:?}");
    assert!(text(&out).contains("started"), "{out:?}");
    assert_eq!(ctl_log(&home), "", "no job to start");
}
