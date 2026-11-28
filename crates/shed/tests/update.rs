//! `self-update` against a local git repo as main and a `file://` release folder: a copy of the shed
//! binary is the live one, and a copy whose build stamp is patched to main's head is the new release.
#![cfg(test)]

mod common;

use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use common::{Server, eventually, temp_dir};
use shed_client::{Calls as _, Conn};
use shed_server::Paths;
use shed_server::proto::{
    BUILD, Dest, ErrorCode, Install, LsParams, Method, Source, Stage, Swap, TerminalInfo, Tool,
};

const SHED: &str = env!("CARGO_BIN_EXE_shed");
const ASSET: &str = if cfg!(target_os = "macos") {
    "apple-darwin"
} else {
    "unknown-linux-gnu"
};

struct Host {
    dir: PathBuf,
    exe: PathBuf,
    main: String,
    serve: Option<Server>,
}

fn run(program: impl AsRef<std::ffi::OsStr>, args: &[&str]) -> Output {
    Command::new(program).args(args).output().expect("runs")
}

fn asset() -> String {
    format!("shed-{}-{ASSET}", std::env::consts::ARCH)
}

impl Host {
    fn new(name: &str) -> Self {
        let dir = temp_dir(name);
        std::fs::create_dir(dir.join("bin")).expect("bin");
        std::fs::create_dir(dir.join("fake-bin")).expect("fake bin");
        for ctl in ["launchctl", "systemctl"] {
            let file = dir.join("fake-bin").join(ctl);
            let script =
                "#!/bin/sh\necho \"$(basename \"$0\") $*\" >> \"$(dirname \"$0\")/ctl.log\"\n";
            std::fs::write(&file, script).expect("fake ctl");
            std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o755)).expect("chmod");
        }
        let exe = dir.join("bin/shed");
        std::fs::copy(SHED, &exe).expect("the live binary");
        let repo = dir.join("repo");
        let repo_s = repo.to_str().expect("utf-8");
        assert!(
            run("git", &["init", "-q", "-b", "main", repo_s])
                .status
                .success()
        );
        let commit = [
            "-C",
            repo_s,
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "commit",
            "-q",
            "--allow-empty",
            "-m",
            "main",
        ];
        assert!(run("git", &commit).status.success());
        let head = run("git", &["-C", repo_s, "rev-parse", "HEAD"]).stdout;
        let main = String::from_utf8_lossy(&head)
            .get(..7)
            .expect("sha")
            .to_owned();
        assert_ne!(main, BUILD);
        Self {
            dir,
            exe,
            main,
            serve: None,
        }
    }

    fn serve(&mut self) {
        self.serve = Some(Server::of(&self.exe, &self.dir, "3600", &[]));
    }

    fn socket(&self) -> PathBuf {
        Paths::in_dir(&self.dir).socket
    }

    fn command(&self, args: &[&str]) -> Command {
        self.command_via(&self.exe, args)
    }

    fn command_via(&self, exe: &Path, args: &[&str]) -> Command {
        let releases = format!("file://{}", self.dir.join("releases").display());
        let mut shed = Command::new(exe);
        shed.arg("--dir").arg(&self.dir).args(args);
        let path = std::env::var("PATH").unwrap_or_default();
        let path = format!("{}:{path}", self.dir.join("fake-bin").display());
        shed.env("HOME", &self.dir)
            .env("XDG_RUNTIME_DIR", &self.dir)
            .env("PATH", path)
            .env_remove("XDG_CONFIG_HOME");
        shed.env("SHED_UPDATE_REPO", self.dir.join("repo"))
            .env("SHED_UPDATE_RELEASES", releases)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        shed
    }

    fn shed(&self, args: &[&str]) -> Output {
        self.command(args).output().expect("shed runs")
    }

    fn release(&self, tag: &str, binary: &Path, sum: Option<&str>) {
        self.release_of(&asset(), tag, binary, sum);
    }

    fn release_of(&self, asset: &str, tag: &str, binary: &Path, sum: Option<&str>) {
        let at = self.dir.join("releases").join(tag);
        std::fs::create_dir_all(&at).expect("release dir");
        std::fs::copy(binary, at.join(asset)).expect("asset");
        let ran = Command::new("sh")
            .args([
                "-c",
                "sha256sum \"$1\" 2>/dev/null || shasum -a 256 \"$1\"",
                "sh",
            ])
            .arg(at.join(asset))
            .output()
            .expect("sha256");
        let said = String::from_utf8_lossy(&ran.stdout);
        let got = said.split_whitespace().next().expect("a sum").to_owned();
        let sums = format!("{}  {asset}\n", sum.unwrap_or(&got));
        std::fs::write(at.join("SHA256SUMS"), sums).expect("SHA256SUMS");
    }

    fn stamped(&self) -> PathBuf {
        let mut bytes = std::fs::read(SHED).expect("the binary");
        let (from, to) = (BUILD.as_bytes(), self.main.as_bytes());
        let mut at = 0;
        while let Some(i) = bytes
            .get(at..)
            .and_then(|b| b.windows(7).position(|w| w == from))
        {
            bytes.splice(at + i..at + i + 7, to.iter().copied());
            at += i + 7;
        }
        let file = self.dir.join("stamped");
        std::fs::write(&file, bytes).expect("stamped copy");
        if cfg!(target_os = "macos") {
            let signed = run(
                "codesign",
                &["-f", "-s", "-", file.to_str().expect("utf-8")],
            );
            assert!(signed.status.success(), "{signed:?}");
        }
        file
    }

    fn version(&self) -> String {
        String::from_utf8_lossy(&run(&self.exe, &["--version"]).stdout)
            .trim()
            .to_owned()
    }

    fn serving(&self) -> String {
        Conn::connect(&self.socket())
            .expect("serve")
            .server()
            .build
            .clone()
    }

    fn terminals(&self) -> Vec<TerminalInfo> {
        let mut client = Conn::connect(&self.socket()).expect("serve");
        client.call(Method::Ls, &LsParams::default()).expect("ls")
    }
}

fn says(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned() + &String::from_utf8_lossy(&out.stderr)
}

#[test]
fn no_release_a_staging_release_a_bad_checksum_and_another_build_change_nothing() {
    let mut host = Host::new("update-refused");
    host.serve = Some(Server::start(&host.dir));
    let before = host.version();
    let unchanged = |host: &Host| {
        assert_eq!(host.version(), before, "the live binary stays");
        assert_eq!(host.serving(), BUILD, "serve stays");
        assert!(!host.exe.with_extension("new").exists(), "nothing staged");
        assert!(!host.exe.with_extension("prev").exists(), "nothing swapped");
    };
    let tag = format!("build-{}", host.main);

    let out = host.shed(&["self-update"]);
    assert!(out.status.success(), "{out:?}");
    assert!(
        says(&out).contains(&format!("waiting for {tag}")),
        "{out:?}"
    );
    unchanged(&host);

    let stamped = host.stamped();
    host.release(&format!("build-staging-{}", host.main), &stamped, None);
    let out = host.shed(&["self-update"]);
    assert!(out.status.success(), "{out:?}");
    let status = std::fs::read_to_string(host.dir.join("update/status.json")).expect("status");
    assert!(status.contains(&format!("waiting for {tag}")), "{status}");
    unchanged(&host);

    host.release(&tag, &stamped, Some(&"0".repeat(64)));
    let out = host.shed(&["self-update"]);
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    assert!(
        says(&out).contains(&format!("rejected: {tag}: UNSTAGED: checksum")),
        "{out:?}"
    );
    unchanged(&host);

    host.release(&tag, Path::new(SHED), None);
    let out = host.shed(&["self-update"]);
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    assert!(says(&out).contains("--version says"), "{out:?}");
    unchanged(&host);
}

#[test]
fn one_pass_installs_mains_release_and_hands_serve_over_keeping_the_terminals() {
    let mut host = Host::new("update-pass");
    host.serve();
    let ran = host.shed(&["run", "--name", "sh", "--", "sh"]);
    assert!(ran.status.success(), "{ran:?}");
    host.shed(&["send", "sh", "printf 'mark-%s\\n' one"]);
    host.shed(&["wait", "sh", "--output", "mark-one", "--timeout", "10"]);
    let pid = host.terminals().first().and_then(|t| t.pid);
    let stamped = host.stamped();
    host.release(&format!("build-{}", host.main), &stamped, None);

    let dry = host.shed(&["self-update", "--dry-run"]);
    assert!(dry.status.success(), "{dry:?}");
    let dry = says(&dry);
    assert!(
        dry.contains(&format!("current  {BUILD}, serve {BUILD}")),
        "{dry}"
    );
    assert!(
        dry.contains(&format!("target   build-{} (main)", host.main)),
        "{dry}"
    );
    assert!(
        dry.contains(&format!("would    fetch {}", asset())),
        "{dry}"
    );
    assert_eq!(host.serving(), BUILD, "a dry run changes nothing");

    assert!(host.shed(&["self-update", "--pause"]).status.success());
    let paused = host.shed(&["self-update"]);
    assert!(says(&paused).contains("paused since"), "{paused:?}");
    assert!(host.version().ends_with(&format!("({BUILD})")));
    assert!(host.shed(&["self-update", "--resume"]).status.success());

    let mut waiting = host.command(&["wait", "sh", "--output", "mark-two", "--timeout", "30"]);
    let waiting = waiting.spawn().expect("a wait across the handover");
    let out = host.shed(&["self-update"]);
    assert!(out.status.success(), "{out:?}");
    let line = format!("updated {BUILD} -> {}; serve handed over", host.main);
    assert!(says(&out).contains(&line), "{out:?}");
    assert!(host.version().ends_with(&format!("({})", host.main)));
    eventually("serve on the new build", || {
        Conn::connect(&host.socket()).is_ok_and(|c| c.server().build == host.main)
    });
    assert_eq!(
        host.serve.as_mut().map(|s| s.0.try_wait().ok().flatten()),
        Some(None)
    );
    let sh = host.terminals();
    assert_eq!(sh.first().and_then(|t| t.pid), pid, "the shell runs on");
    let read = says(&host.shed(&["read", "sh"]));
    assert!(read.contains("mark-one"), "{read}");
    host.shed(&["send", "sh", "printf 'mark-%s\\n' two"]);
    let waited = waiting.wait_with_output().expect("the wait ends");
    assert!(waited.status.success(), "the wait reconnected: {waited:?}");

    let again = host.shed(&["self-update"]);
    assert!(again.status.success(), "{again:?}");
    assert!(
        says(&again).contains(&format!("up to date on {}", host.main)),
        "{again:?}"
    );
}

#[test]
fn a_pass_through_a_symlink_swaps_its_target_and_keeps_the_symlink() {
    let mut host = Host::new("update-symlink");
    host.serve();
    let stamped = host.stamped();
    host.release(&format!("build-{}", host.main), &stamped, None);
    std::fs::create_dir(host.dir.join("link")).expect("link dir");
    let link = host.dir.join("link/shed");
    std::os::unix::fs::symlink(&host.exe, &link).expect("symlink");

    let out = host
        .command_via(&link, &["self-update"])
        .output()
        .expect("runs");
    assert!(out.status.success(), "{out:?}");
    let line = format!("updated {BUILD} -> {}; serve handed over", host.main);
    assert!(says(&out).contains(&line), "{out:?}");
    assert_eq!(
        std::fs::read_link(&link).expect("still a symlink"),
        host.exe
    );
    assert!(host.version().ends_with(&format!("({})", host.main)));
    let prev = host.exe.with_extension("prev");
    let prev_meta = std::fs::symlink_metadata(&prev).expect("prev kept");
    assert!(prev_meta.is_file(), "prev is the old binary, not a symlink");
    assert!(
        !link.with_extension("prev").exists(),
        "nothing beside the link"
    );
    eventually("serve on the new build", || {
        Conn::connect(&host.socket()).is_ok_and(|c| c.server().build == host.main)
    });
}

#[test]
fn a_pass_from_another_binary_than_serves_names_both_and_swaps_nothing() {
    let mut host = Host::new("update-other");
    host.serve = Some(Server::start(&host.dir));
    let stamped = host.stamped();
    host.release(&format!("build-{}", host.main), &stamped, None);
    let before = host.version();

    let out = host.shed(&["self-update"]);
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    let said = says(&out);
    let serve = std::fs::canonicalize(SHED).expect("serve's binary");
    assert!(said.contains(&serve.display().to_string()), "{said}");
    assert!(said.contains(&host.exe.display().to_string()), "{said}");
    assert!(!said.contains("pending"), "{said}");
    assert_eq!(host.version(), before, "the live binary stays");
    assert_eq!(host.serving(), BUILD, "serve stays");
}

fn update_jobs(dir: &Path, label: &str) -> Vec<PathBuf> {
    if cfg!(target_os = "macos") {
        vec![dir.join(format!("Library/LaunchAgents/{label}.plist"))]
    } else {
        let units = dir.join(".config/systemd/user");
        ["service", "timer"]
            .map(|ext| units.join(format!("{label}.{ext}")))
            .into()
    }
}

#[test]
fn a_pass_from_a_build_with_the_old_job_names_leaves_only_the_dev_shed_update_job() {
    let host = Host::new("update-jobs");
    let old = update_jobs(&host.dir, "shed-update");
    std::fs::create_dir_all(old.first().and_then(|f| f.parent()).expect("dir")).expect("job dir");
    for file in &old {
        std::fs::write(file, "old\n").expect("old job");
    }
    let stamped = host.stamped();
    host.release(&format!("build-{}", host.main), &stamped, None);

    let out = host.shed(&["self-update"]);
    assert!(out.status.success(), "{out:?}");
    let line = format!("updated {BUILD} -> {}", host.main);
    assert!(says(&out).contains(&line), "{out:?}");
    for file in &old {
        assert!(!file.exists(), "{} removed", file.display());
    }
    let exe = host.exe.display();
    for file in update_jobs(&host.dir, "dev.shed.update") {
        let text = std::fs::read_to_string(&file).expect("the dev.shed.update job");
        assert!(
            text.contains(&format!("{exe}</string>\n    <string>self-update<"))
                || text.contains(&format!("ExecStart={exe} self-update\n"))
                || text.contains("OnUnitActiveSec=300\n"),
            "{text}"
        );
    }
    assert!(
        !update_jobs(&host.dir, "dev.shed.serve")
            .iter()
            .any(|f| f.exists()),
        "a pass leaves the serve job alone"
    );
}

fn script(path: &Path, says: &str) {
    std::fs::write(path, format!("#!/bin/sh\necho '{says}'\n")).expect("script");
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).expect("chmod");
}

const OLD: &str = "drover 1.0 (0ld1234)";
const NEW: &str = "drover 1.1 (9e45678)";

struct Tools {
    host: Host,
    _serve: Server,
    exe: PathBuf,
    other: PathBuf,
    asset: String,
    caller: Conn,
}

impl Tools {
    fn new(name: &str) -> Self {
        let host = Host::new(name);
        let serve = Server::start(&host.dir);
        std::fs::create_dir(host.dir.join("tools")).expect("tools");
        let (exe, other) = (host.dir.join("tools/drover"), host.dir.join("tools/cooee"));
        script(&exe, OLD);
        script(&other, "cooee 1.0");
        let (made, mute) = (host.dir.join("made"), host.dir.join("mute"));
        script(&made, NEW);
        std::fs::write(&mute, "#!/bin/sh\nexit 1\n").expect("mute");
        std::fs::set_permissions(&mute, std::fs::Permissions::from_mode(0o755)).expect("chmod");
        let asset = format!("drover-{}-{ASSET}", std::env::consts::ARCH);
        host.release_of(&asset, "build-9e45678", &made, None);
        host.release_of(&asset, "build-bad", &made, Some(&"0".repeat(64)));
        host.release_of(&asset, "build-mute", &mute, None);
        let tool = Tool {
            name: "drover".into(),
            exe: exe.clone(),
        };
        let caller = Conn::connect_as(&host.socket(), tool).expect("hello naming drover");
        Self {
            host,
            _serve: serve,
            exe,
            other,
            asset,
            caller,
        }
    }

    fn source(&self, tool: &str, tag: &str) -> Source {
        let releases = format!("file://{}", self.host.dir.join("releases").display());
        Source {
            tool: tool.into(),
            releases,
            tag: tag.into(),
        }
    }

    fn stage(&self, tag: &str, dest: &Path) -> Stage {
        let source = self.source("drover", tag);
        Stage {
            source,
            build: "9e45678".into(),
            dest: dest.to_owned(),
        }
    }

    fn dest(&self) -> Dest {
        Dest {
            dest: self.exe.clone(),
        }
    }
}

#[test]
fn install_refuses_any_binary_but_sheds_own_and_the_callers_named_tool() {
    let t = Tools::new("install-refused");
    let refused =
        |e: shed_client::Error| assert_eq!(e.code(), Some(ErrorCode::NotOwnBinary), "{e}");
    let cooee = Dest {
        dest: t.other.clone(),
    };
    refused(t.caller.install_ls(&cooee).expect_err("cooee is not ours"));
    refused(t.caller.install_back(&cooee).expect_err("nor to put back"));
    let theirs = t.source("cooee", "build-9e45678");
    refused(t.caller.install_find(&theirs).expect_err("nor its release"));
    let over = t.stage("build-9e45678", &t.other);
    refused(
        t.caller
            .install_stage(&over)
            .expect_err("drover's release over cooee"),
    );
    let anonymous = Conn::connect(&t.host.socket()).expect("hello");
    refused(
        anonymous
            .install_ls(&t.dest())
            .expect_err("a client that named no tool"),
    );
    assert_eq!(
        std::fs::read_dir(t.host.dir.join("tools"))
            .expect("tools")
            .count(),
        2
    );

    let found = t
        .caller
        .install_find(&t.source("drover", "build-9e45678"))
        .expect("find");
    assert!(found.assets.contains_key(&t.asset), "{found:?}");
    let missing = t.caller.install_find(&t.source("drover", "build-none"));
    let missing = missing.expect_err("no release");
    assert_eq!(missing.code(), Some(ErrorCode::NoRelease), "{missing}");

    let ls = t.host.shed(&["install", "ls"]);
    assert!(ls.status.success(), "{ls:?}");
    let said = says(&ls);
    assert!(
        said.contains(&format!("live    {}", t.host.version())),
        "{said}"
    );
    assert!(
        said.contains("staged  -") && said.contains("prev    -"),
        "{said}"
    );
}

#[test]
fn stage_swap_and_back_keep_the_old_binary_until_the_new_one_answers() {
    let t = Tools::new("install-tool");
    let only_live = Install {
        live: Some(OLD.into()),
        ..Install::default()
    };
    for (tag, check) in [("build-bad", "checksum"), ("build-mute", "version")] {
        let e = t
            .caller
            .install_stage(&t.stage(tag, &t.exe))
            .expect_err(tag);
        assert_eq!(e.code(), Some(ErrorCode::Unstaged), "{e}");
        assert!(e.to_string().contains(&format!("UNSTAGED: {check}")), "{e}");
        assert_eq!(t.caller.install_ls(&t.dest()).expect("ls"), only_live);
        assert!(
            !t.host.dir.join("tools/drover.new.part").exists(),
            "no part file left"
        );
    }
    let staged = t
        .caller
        .install_stage(&t.stage("build-9e45678", &t.exe))
        .expect("stage");
    assert_eq!(
        staged,
        Install {
            staged: Some(NEW.into()),
            ..only_live.clone()
        }
    );

    let wrong = Swap {
        dest: t.exe.clone(),
        build: "abcdef0".into(),
    };
    let wrong = t.caller.install_swap(&wrong).expect_err("another build");
    assert_eq!(wrong.code(), Some(ErrorCode::Unstaged), "{wrong}");
    assert_eq!(
        t.caller.install_ls(&t.dest()).expect("ls"),
        staged,
        "nothing moved"
    );
    let swap = Swap {
        dest: t.exe.clone(),
        build: "9e45678".into(),
    };
    let swapped = t.caller.install_swap(&swap).expect("swap");
    let want = Install {
        live: Some(NEW.into()),
        staged: None,
        prev: Some(OLD.into()),
    };
    assert_eq!(swapped, want);
    assert_eq!(t.caller.install_back(&t.dest()).expect("back"), only_live);
    let again = t.caller.install_back(&t.dest()).expect_err("no prev left");
    assert_eq!(again.code(), Some(ErrorCode::Unstaged), "{again}");
}
