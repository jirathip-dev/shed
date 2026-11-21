use std::env::consts::ARCH;
use std::ffi::OsStr;
use std::fs;
use std::io::IsTerminal as _;
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use anyhow::{Context as _, bail};
use serde_json::{Value, json};
use shed_client::Conn;
use shed_server::proto::{BUILD, Handover, Method};
use shed_server::runner::{self, ExitCode};
use shed_server::{Paths, clock, config};

fn main_head(repo: &str) -> anyhow::Result<String> {
    let ran = runner::output("git", ["ls-remote", repo, "refs/heads/main"], &[])?;
    let (said, err) = (
        String::from_utf8_lossy(&ran.stdout),
        String::from_utf8_lossy(&ran.stderr),
    );
    let sha = (said.lines()).find_map(|l| l.strip_suffix("\trefs/heads/main"));
    match sha.filter(|s| s.len() == 40 && s.bytes().all(|b| b.is_ascii_hexdigit())) {
        Some(sha) if ran.status.success() => Ok(sha.get(..7).unwrap_or_default().to_owned()),
        _ => bail!("git ls-remote {repo} main: {}", err.trim()),
    }
}

struct Staged(PathBuf);

impl Drop for Staged {
    fn drop(&mut self) {
        fs::remove_file(&self.0).ok();
    }
}

fn curl(url: &str) -> Result<Vec<u8>, String> {
    let ran = runner::output("curl", ["-fsSL", url], &[]).map_err(|e| format!("curl: {e}"))?;
    let err = String::from_utf8_lossy(&ran.stderr).trim().to_owned();
    ran.status.success().then_some(ran.stdout).ok_or(err)
}

#[cfg(target_os = "macos")]
const OS: &str = "apple-darwin";
#[cfg(not(target_os = "macos"))]
const OS: &str = "unknown-linux-gnu";

impl Staged {
    fn fetch(url: &str, sums: &str, to: &str, exe: &Path) -> anyhow::Result<Self> {
        let asset = format!("shed-{ARCH}-{OS}");
        let want = sums.lines().find_map(|l| {
            let (sum, name) = l.split_once(char::is_whitespace)?;
            (name.trim_start().trim_start_matches('*') == asset).then_some(sum)
        });
        let want = want.with_context(|| format!("SHA256SUMS names no {asset}"))?;
        let staged = Self(exe.with_extension("new"));
        let bytes = curl(&format!("{url}/{asset}")).map_err(anyhow::Error::msg)?;
        fs::write(&staged.0, bytes)?;
        let sum = [
            "-c",
            "sha256sum \"$1\" 2>/dev/null || shasum -a 256 \"$1\"",
            "sh",
        ];
        let args = sum
            .map(OsStr::new)
            .into_iter()
            .chain([staged.0.as_os_str()]);
        let ran = runner::output("sh", args, &[])?;
        let got = String::from_utf8_lossy(&ran.stdout);
        let got = got.split_whitespace().next().unwrap_or_default();
        if got != want {
            bail!("checksum: {asset} is {got:?}, SHA256SUMS says {want}");
        }
        fs::set_permissions(&staged.0, fs::Permissions::from_mode(0o755))?;
        let ran = runner::output(&staged.0, ["--version"], &[])?;
        let said = String::from_utf8_lossy(&ran.stdout);
        if !said.trim_end().ends_with(&format!("({to})")) {
            bail!("--version says {:?}, not {to}", said.trim());
        }
        Ok(staged)
    }

    fn swap(self, exe: &Path) -> anyhow::Result<()> {
        let prev = exe.with_extension("prev");
        fs::remove_file(&prev).ok();
        fs::hard_link(exe, &prev).with_context(|| prev.display().to_string())?;
        fs::rename(&self.0, exe).with_context(|| exe.display().to_string())?;
        Ok(())
    }
}

fn hand_over(serve: Option<Conn>, to: &str) -> String {
    match serve.map(|mut c| c.call::<_, Handover>(Method::Handover, &json!({"build": to}))) {
        None => "not running".to_owned(),
        Some(Ok(_)) => "handed over".to_owned(),
        Some(Err(e)) => format!("pending ({e}); the next pass asks again"),
    }
}

fn pass(paths: &Paths, dir: &Path, exe: &Path, dry: bool) -> anyhow::Result<String> {
    if let Ok(meta) = fs::metadata(dir.join("paused")) {
        let since = meta.modified()?.duration_since(UNIX_EPOCH)?.as_secs();
        return Ok(format!("paused since {since}; shed self-update --resume"));
    }
    let (repo, releases) = config::update_source();
    let to = main_head(&repo)?;
    let tag = format!("build-{to}");
    if dry {
        crate::out::line(format_args!("target   {tag} (main)"));
    }
    let mut serve = Conn::connect(&paths.socket).ok();
    let serving = serve.as_ref().map(|c| c.server().build.clone());
    if to == BUILD {
        return Ok(match serving.filter(|b| b != BUILD) {
            None => format!("up to date on {BUILD}"),
            Some(b) if dry => format!("would    hand serve over from {b}"),
            Some(_) => format!("serve {} on {BUILD}", hand_over(serve, BUILD)),
        });
    }
    let url = format!("{releases}/{tag}");
    let sums = match curl(&format!("{url}/SHA256SUMS")) {
        Ok(sums) => String::from_utf8_lossy(&sums).into_owned(),
        Err(why) => return Ok(format!("waiting for {tag}: {why}")),
    };
    if dry {
        return Ok(format!(
            "would    fetch shed-{ARCH}-{OS}, check, swap, hand over"
        ));
    }
    let staged = Staged::fetch(&url, &sums, &to, exe).context(tag.clone())?;
    let check = json!({"build": to, "candidate": staged.0});
    let checked = serve
        .as_mut()
        .map(|c| c.call::<_, Handover>(Method::HandoverCheck, &check));
    if let Some(Err(e)) = checked {
        return Ok(format!("deferred {tag}: {e}; nothing swapped"));
    }
    staged.swap(exe)?;
    let served = hand_over(serve, &to);
    Ok(format!("updated {BUILD} -> {to}; serve {served}"))
}

pub fn last(paths: &Paths) -> Option<String> {
    let text = fs::read(paths.state.with_file_name("update/status.json")).ok()?;
    let status: Value = serde_json::from_slice(&text).ok()?;
    status.get("why").and_then(Value::as_str).map(str::to_owned)
}

#[derive(clap::Args)]
pub struct Args {
    #[arg(long, conflicts_with_all = ["pause", "resume"])]
    dry_run: bool,
    #[arg(long, conflicts_with = "resume")]
    pause: bool,
    #[arg(long)]
    resume: bool,
}

pub fn self_update(paths: &Paths, args: &Args) -> anyhow::Result<ExitCode> {
    let dir = paths.state.with_file_name("update");
    fs::create_dir_all(&dir).with_context(|| dir.display().to_string())?;
    let marker = dir.join("paused");
    if args.pause {
        fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&marker)?;
        crate::out::line("paused");
        return Ok(ExitCode::SUCCESS);
    } else if args.resume {
        fs::remove_file(&marker).ok();
        crate::out::line("resumed");
        return Ok(ExitCode::SUCCESS);
    }
    let lock = fs::File::create(dir.join("lock"))?;
    if lock.try_lock().is_err() {
        crate::out::line("another self-update pass runs");
        return Ok(ExitCode::SUCCESS);
    }
    let exe = std::env::current_exe().context("this binary's path")?;
    let dry = args.dry_run;
    if dry {
        let serve = Conn::connect(&paths.socket).map(|c| c.server().build.clone());
        let serve = serve.unwrap_or_else(|_| "not running".to_owned());
        crate::out::line(format_args!("current  {BUILD}, serve {serve}"));
    }
    let (ok, line) = match pass(paths, &dir, &exe, dry) {
        Ok(line) => (true, line),
        Err(e) => (false, format!("rejected: {e:#}; the live binary stays")),
    };
    if dry || std::io::stdout().is_terminal() || last(paths).is_none_or(|why| why != line) {
        crate::out::line(&line);
    }
    if !dry {
        let status = json!({"at": clock::unix_now(), "current": BUILD, "ok": ok, "why": line});
        fs::write(dir.join("status.json"), status.to_string())?;
    }
    Ok(if ok {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}
