//! Installs of shed's own binary and the calling tool's on the main host: find, stage, swap, back and ls.

use std::ffi::OsString;
use std::fs;
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};

use shed_core::runner;
use shed_proto::{ErrorCode, Install, Release, RpcError, Source, Stage, Swap, Tool};

#[cfg(target_os = "macos")]
const OS: &str = "apple-darwin";
#[cfg(not(target_os = "macos"))]
const OS: &str = "unknown-linux-gnu";

/// `tool`'s release asset for this machine: `<tool>-<arch>-<os>`.
#[must_use]
pub fn asset(tool: &str) -> String {
    format!("{tool}-{}-{OS}", std::env::consts::ARCH)
}

fn beside(dest: &Path, ext: &str) -> PathBuf {
    let mut path = OsString::from(dest);
    path.push(ext);
    path.into()
}

fn unstaged(check: &str, why: impl std::fmt::Display) -> RpcError {
    RpcError::new(ErrorCode::Unstaged, format!("{check}: {why}"))
}

fn io(path: &Path) -> impl Fn(std::io::Error) -> RpcError + '_ {
    move |e| RpcError::new(ErrorCode::Io, format!("{}: {e}", path.display()))
}

/// Refuses unless `tool` and `dest`, each where given, are shed's own binary or `caller`'s named one.
///
/// # Errors
/// `NOT_OWN_BINARY`.
pub fn own(caller: Option<&Tool>, tool: Option<&str>, dest: Option<&Path>) -> Result<(), RpcError> {
    let real = |p: &Path| fs::canonicalize(p).unwrap_or_else(|_| p.to_owned());
    let shed = std::env::current_exe().ok().map(|exe| ("shed", exe));
    let caller = caller.map(|t| (t.name.as_str(), t.exe.clone()));
    let mut ours = shed.into_iter().chain(caller);
    if ours.any(|(name, exe)| {
        tool.is_none_or(|t| t == name) && dest.is_none_or(|d| real(d) == real(&exe))
    }) {
        return Ok(());
    }
    let named = dest.map_or_else(
        || tool.unwrap_or_default().to_owned(),
        |d| d.display().to_string(),
    );
    Err(RpcError::new(
        ErrorCode::NotOwnBinary,
        format!("{named} is neither shed's own binary nor the tool this client named at hello"),
    ))
}

fn curl(url: &str) -> Result<Vec<u8>, String> {
    let ran = runner::output("curl", ["-fsSL", url], &[]).map_err(|e| format!("curl: {e}"))?;
    let err = String::from_utf8_lossy(&ran.stderr).trim().to_owned();
    ran.status.success().then_some(ran.stdout).ok_or(err)
}

/// The release's assets from its `SHA256SUMS`.
///
/// # Errors
/// `NO_RELEASE` when it cannot be read.
pub fn find(src: &Source) -> Result<Release, RpcError> {
    let sums = curl(&format!("{}/{}/SHA256SUMS", src.releases, src.tag));
    let sums =
        sums.map_err(|why| RpcError::new(ErrorCode::NoRelease, format!("{}: {why}", src.tag)))?;
    let assets = (String::from_utf8_lossy(&sums).lines())
        .filter_map(|l| {
            let (sum, name) = l.split_once(char::is_whitespace)?;
            Some((
                name.trim_start().trim_start_matches('*').to_owned(),
                sum.to_owned(),
            ))
        })
        .collect();
    Ok(Release {
        tag: src.tag.clone(),
        assets,
    })
}

fn version(exe: &Path) -> Option<String> {
    let ran = runner::output(exe, ["--version"], &[]).ok()?;
    let said = String::from_utf8_lossy(&ran.stdout).trim().to_owned();
    ran.status.success().then_some(said)
}

fn is(exe: &Path, build: &str) -> Result<(), String> {
    match version(exe) {
        Some(said) if said.contains(build) => Ok(()),
        said => Err(format!(
            "{} --version says {said:?}, not {build}",
            exe.display()
        )),
    }
}

/// The `--version` line of `dest`, `dest.new` and `dest.prev`.
#[must_use]
pub fn ls(dest: &Path) -> Install {
    Install {
        live: version(dest),
        staged: version(&beside(dest, ".new")),
        prev: version(&beside(dest, ".prev")),
    }
}

fn sha256(file: &Path) -> Result<String, String> {
    let script = "sha256sum \"$1\" 2>/dev/null || shasum -a 256 \"$1\"";
    let args = [
        OsString::from("-c"),
        script.into(),
        "sh".into(),
        file.into(),
    ];
    let ran = runner::output("sh", args, &[]).map_err(|e| e.to_string())?;
    let said = String::from_utf8_lossy(&ran.stdout);
    Ok(said
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .to_owned())
}

fn fetch(p: &Stage, part: &Path) -> Result<(), RpcError> {
    let asset = asset(&p.source.tool);
    let release = find(&p.source)?;
    let want = release.assets.get(&asset);
    let want = want.ok_or_else(|| unstaged("asset", format!("SHA256SUMS names no {asset}")))?;
    let url = format!("{}/{}/{asset}", p.source.releases, p.source.tag);
    let bytes = curl(&url).map_err(|e| unstaged("download", e))?;
    fs::write(part, bytes).map_err(|e| unstaged("download", e))?;
    let got = sha256(part).map_err(|e| unstaged("checksum", e))?;
    if got != *want {
        let why = format!("{asset} is {got:?}, SHA256SUMS says {want}");
        return Err(unstaged("checksum", why));
    }
    fs::set_permissions(part, fs::Permissions::from_mode(0o755)).map_err(io(part))?;
    is(part, &p.build).map_err(|e| unstaged("version", e))
}

/// Fetches and checks the release into `dest.new`, leaving the live binary alone.
///
/// # Errors
/// `NO_RELEASE`, `UNSTAGED` naming the failed check (the part file removed), `IO`.
pub fn stage(p: &Stage) -> Result<Install, RpcError> {
    let new = beside(&p.dest, ".new");
    if is(&new, &p.build).is_err() {
        let part = beside(&p.dest, ".new.part");
        let fetched = fetch(p, &part).and_then(|()| fs::rename(&part, &new).map_err(io(&new)));
        if fetched.is_err() {
            fs::remove_file(&part).ok();
        }
        fetched?;
    }
    Ok(ls(&p.dest))
}

/// Keeps the live binary as `dest.prev` and renames `dest.new` over it, once `dest.new` answers as `build`.
///
/// # Errors
/// `UNSTAGED` when `dest.new` is not `build`, `IO`.
pub fn swap(p: &Swap) -> Result<Install, RpcError> {
    if is(&p.dest, &p.build).is_ok() {
        return Ok(ls(&p.dest));
    }
    let new = beside(&p.dest, ".new");
    is(&new, &p.build).map_err(|e| unstaged("version", e))?;
    let prev = beside(&p.dest, ".prev");
    fs::remove_file(&prev).ok();
    fs::hard_link(&p.dest, &prev).map_err(io(&prev))?;
    fs::rename(&new, &p.dest).map_err(io(&p.dest))?;
    Ok(ls(&p.dest))
}

/// Renames `dest.prev` over the live binary, once `dest.prev` answers.
///
/// # Errors
/// `UNSTAGED` when `dest.prev` does not answer, `IO`.
pub fn back(dest: &Path) -> Result<Install, RpcError> {
    let prev = beside(dest, ".prev");
    if version(&prev).is_none() {
        return Err(unstaged(
            "version",
            format!("{} does not answer", prev.display()),
        ));
    }
    fs::rename(&prev, dest).map_err(io(dest))?;
    Ok(ls(dest))
}
