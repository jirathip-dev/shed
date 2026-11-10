//! The installed ACP agents: `agents.json` next to the state file, each pinned to the version the ACP
//! registry listed when it was added. Nothing upgrades an agent; `rm` and `add` again does.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::Error;

/// The ACP registry `add` reads by default.
pub const REGISTRY: &str = "https://cdn.agentclientprotocol.com/registry/v1/latest/registry.json";

/// One installed agent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Agent {
    /// The registry id.
    pub id: String,
    /// The pinned version.
    pub version: String,
    /// The distribution it came from: `binary`, `npx` or `uvx`.
    pub distribution: String,
    /// The ACP adapter's command line (`env K=V …` first when the registry sets variables).
    pub cmd: Vec<String>,
}

#[derive(Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct AgentsFile {
    agents: Vec<Agent>,
}

fn fail(e: impl std::fmt::Display) -> Error {
    Error::Agents(e.to_string())
}

fn safe(s: &str) -> bool {
    !s.is_empty()
        && !s.starts_with('.')
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

/// `agents.json` beside the state file `state`.
#[must_use]
pub fn file(state: &Path) -> PathBuf {
    state.with_file_name("agents.json")
}

/// The installed agents, by id; none when `agents.json` does not exist.
///
/// # Errors
/// [`Error::Io`] or [`Error::Json`] when the file cannot be read.
pub fn list(state: &Path) -> Result<Vec<Agent>, Error> {
    match fs::read(file(state)) {
        Ok(bytes) => Ok(serde_json::from_slice::<AgentsFile>(&bytes)?.agents),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(e.into()),
    }
}

fn save(state: &Path, mut agents: Vec<Agent>) -> Result<(), Error> {
    agents.sort_by(|a, b| a.id.cmp(&b.id));
    let path = file(state);
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("json.tmp");
    let mut bytes = serde_json::to_vec_pretty(&AgentsFile { agents })?;
    bytes.push(b'\n');
    fs::write(&tmp, bytes)?;
    Ok(fs::rename(&tmp, &path)?)
}

fn run(program: &str, args: &[&Path]) -> Result<Vec<u8>, Error> {
    let cmd = format!("{program} {args:?}");
    let out =
        shed_core::runner::output(program, args, &[]).map_err(|e| fail(format!("{cmd}: {e}")))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        return Err(fail(format!("{cmd} failed: {}", err.trim())));
    }
    Ok(out.stdout)
}

fn fetch(source: &str) -> Result<Vec<u8>, Error> {
    if source.starts_with("http://") || source.starts_with("https://") {
        run(
            "curl",
            &["-fsSL", "--max-time", "60", source].map(Path::new),
        )
    } else {
        Ok(fs::read(source)?)
    }
}

fn platform() -> String {
    let os = match std::env::consts::OS {
        "macos" => "darwin",
        os => os,
    };
    format!("{os}-{}", std::env::consts::ARCH)
}

fn strings(v: &Value) -> Vec<String> {
    v.as_array()
        .into_iter()
        .flatten()
        .filter_map(|s| s.as_str().map(str::to_owned))
        .collect()
}

fn with_env(entry: &Value, cmd: Vec<String>) -> Vec<String> {
    let Some(env) = entry["env"].as_object().filter(|e| !e.is_empty()) else {
        return cmd;
    };
    let vars = env
        .iter()
        .map(|(k, v)| format!("{k}={}", v.as_str().unwrap_or_default()));
    std::iter::once("env".to_owned())
        .chain(vars)
        .chain(cmd)
        .collect()
}

fn install_binary(entry: &Value, dir: &Path) -> Result<Vec<String>, Error> {
    let url = entry["archive"]
        .as_str()
        .ok_or_else(|| fail("no archive"))?;
    let archive = dir.join(url.rsplit('/').next().unwrap_or("archive"));
    fs::create_dir_all(dir)?;
    run(
        "curl",
        &["-fsSL".as_ref(), "-o".as_ref(), &archive, url.as_ref()],
    )?;
    if let Some(want) = entry["sha256"].as_str() {
        let sum = run("shasum", &["-a".as_ref(), "256".as_ref(), &archive])
            .or_else(|_| run("sha256sum", &[&archive]))?;
        let have = String::from_utf8_lossy(&sum);
        if have.split_whitespace().next() != Some(want) {
            let _ = fs::remove_file(&archive);
            return Err(fail(format!("{url}: sha256 mismatch")));
        }
    }
    if Path::new(url)
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("zip"))
    {
        run("unzip", &["-oq".as_ref(), &archive, "-d".as_ref(), dir])?;
    } else {
        run("tar", &["-xf".as_ref(), &archive, "-C".as_ref(), dir])?;
    }
    fs::remove_file(&archive)?;
    let cmd = entry["cmd"].as_str().ok_or_else(|| fail("no cmd"))?;
    let program = dir.join(cmd.trim_start_matches("./"));
    let program = program.to_string_lossy().into_owned();
    Ok(std::iter::once(program)
        .chain(strings(&entry["args"]))
        .collect())
}

/// Installs agent `id` at the version the registry at `source` (a URL or a file) lists: this platform's
/// binary (unpacked under `agents/<id>/<version>/` beside the state file), else `npx`, else `uvx`.
///
/// # Errors
/// [`Error::Agents`] when it is installed already, the registry has no such agent or no distribution
/// shed runs here, or a download, checksum or unpack fails.
pub fn add(state: &Path, id: &str, source: &str) -> Result<Agent, Error> {
    if !safe(id) {
        return Err(fail(format!("bad agent id {id:?}")));
    }
    let mut agents = list(state)?;
    if let Some(a) = agents.iter().find(|a| a.id == id) {
        return Err(fail(format!(
            "{id} is installed at {}; rm it first to change versions",
            a.version
        )));
    }
    let registry: Value = serde_json::from_slice(&fetch(source)?)?;
    let entry = registry["agents"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|a| a["id"] == id)
        .ok_or_else(|| fail(format!("no agent {id:?} in {source}")))?;
    let version = entry["version"].as_str().unwrap_or_default().to_owned();
    if !safe(&version) {
        return Err(fail(format!("{id}: bad version {version:?}")));
    }
    let dist = &entry["distribution"];
    let binary = &dist["binary"][platform()];
    let (distribution, cmd) = if binary.is_object() {
        let dir = state.with_file_name("agents").join(id).join(&version);
        ("binary", with_env(binary, install_binary(binary, &dir)?))
    } else if let Some(package) = dist["npx"]["package"].as_str() {
        let cmd = ["npx", "-y", package].map(str::to_owned).to_vec();
        let cmd = [cmd, strings(&dist["npx"]["args"])].concat();
        ("npx", with_env(&dist["npx"], cmd))
    } else if let Some(package) = dist["uvx"]["package"].as_str() {
        let cmd = [
            vec!["uvx".to_owned(), package.to_owned()],
            strings(&dist["uvx"]["args"]),
        ];
        ("uvx", with_env(&dist["uvx"], cmd.concat()))
    } else {
        return Err(fail(format!(
            "{id} has no binary for {}, npx or uvx",
            platform()
        )));
    };
    let agent = Agent {
        id: id.to_owned(),
        version,
        distribution: distribution.to_owned(),
        cmd,
    };
    agents.push(agent.clone());
    save(state, agents)?;
    Ok(agent)
}

/// Removes agent `id` (and its unpacked binary, if any).
///
/// # Errors
/// [`Error::Agents`] when it is not installed.
pub fn rm(state: &Path, id: &str) -> Result<Agent, Error> {
    let mut agents = list(state)?;
    let at = agents
        .iter()
        .position(|a| a.id == id)
        .ok_or_else(|| fail(format!("{id} is not installed")))?;
    let agent = agents.remove(at);
    save(state, agents)?;
    let dir = state.with_file_name("agents").join(id);
    if safe(id) && dir.is_dir() {
        fs::remove_dir_all(&dir)?;
    }
    Ok(agent)
}
