//! `self-update` against a local git repo as main and a `file://` release folder: a copy of the shed
//! binary is the live one, and a copy whose build stamp is patched to main's head is the new release.
#![cfg(test)]

mod common;

use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use common::{Server, eventually, temp_dir};
use shed_server::proto::{BUILD, LsParams, Method, TerminalInfo};
use shed_server::{Client, Paths};

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
        let releases = format!("file://{}", self.dir.join("releases").display());
        let mut shed = Command::new(&self.exe);
        shed.arg("--dir").arg(&self.dir).args(args);
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
        let at = self.dir.join("releases").join(tag);
        std::fs::create_dir_all(&at).expect("release dir");
        std::fs::copy(binary, at.join(asset())).expect("asset");
        let ran = Command::new("sh")
            .args([
                "-c",
                "sha256sum \"$1\" 2>/dev/null || shasum -a 256 \"$1\"",
                "sh",
            ])
            .arg(at.join(asset()))
            .output()
            .expect("sha256");
        let said = String::from_utf8_lossy(&ran.stdout);
        let got = said.split_whitespace().next().expect("a sum").to_owned();
        let sums = format!("{}  {}\n", sum.unwrap_or(&got), asset());
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
        Client::connect(&self.socket())
            .expect("serve")
            .server()
            .build
            .clone()
    }

    fn terminals(&self) -> Vec<TerminalInfo> {
        let mut client = Client::connect(&self.socket()).expect("serve");
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
        says(&out).contains(&format!("rejected: {tag}: checksum")),
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
        Client::connect(&host.socket()).is_ok_and(|c| c.server().build == host.main)
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
