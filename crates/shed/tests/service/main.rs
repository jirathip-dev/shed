//! `shed install|uninstall|doctor` with `HOME` in a temp dir: the job file they write and check. A fake
//! `launchctl` and `systemctl` on `PATH` log what install runs, so no test touches the real job.
#![cfg(test)]

mod autostart;
#[path = "../common/mod.rs"]
mod common;
mod doctor;
mod install;

use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const SHED: &str = env!("CARGO_BIN_EXE_shed");

fn shed(home: &Path, args: &[&str]) -> Output {
    shed_with(home, args, &[])
}

fn shed_with(home: &Path, args: &[&str], env: &[(&str, &str)]) -> Output {
    let bin = home.join("fake-bin");
    std::fs::create_dir_all(&bin).expect("fake bin");
    for ctl in ["launchctl", "systemctl"] {
        let file = bin.join(ctl);
        let log = "echo \"$(basename \"$0\") $*\" >> \"$(dirname \"$0\")/ctl.log\"\n\
                   [ -z \"$CTL_FAIL\" ] || case \"$*\" in *\"$CTL_FAIL\"*) exit 3; esac\n\
                   case \"$*\" in *\" show \"*) printf 'ActiveState=active\\n\\nActiveState=active\\n'; esac\n\
                   case \"$*\" in *\" start \"*|kickstart*) \"$SHED_BIN\" serve </dev/null >/dev/null 2>&1 & esac\n";
        std::fs::write(&file, format!("#!/bin/sh\n{log}")).expect("fake ctl");
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    }
    let path = std::env::var("PATH").unwrap_or_default();
    Command::new(SHED)
        .env("HOME", home)
        .env("XDG_RUNTIME_DIR", home)
        .env("SHED_BIN", SHED)
        .env("SHELL", "/bin/sh")
        .env("PATH", format!("{}:{path}", bin.display()))
        .env_remove("XDG_CONFIG_HOME")
        .envs(env.iter().copied())
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
        home.join(".config/systemd/user/dev.shed.serve.service")
    }
}

fn uid() -> String {
    let uid = Command::new("id").arg("-u").output().expect("id -u").stdout;
    String::from_utf8_lossy(&uid).trim().to_owned()
}
