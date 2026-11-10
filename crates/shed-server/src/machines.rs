//! Machines over SSH: the `[machines]` table of the config file and a [`Client`] through
//! `ssh <target> shed connect`.
//!
//! The config file is a small TOML subset, written by `shed machine add|rm`:
//!
//! ```toml
//! [machines]
//! box = "user@box.example"
//! ```
//!
//! Names are letters, digits, `-` and `_`; targets are plain strings without quotes, backslashes or
//! spaces. Anything else is an error that names the line.

use std::fmt::Write as _;
use std::fs;
use std::io;
use std::os::fd::OwnedFd;
use std::os::unix::fs::DirBuilderExt as _;
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::time::Duration;

use serde::Serialize;

use crate::proto::{ErrorCode, MachineStats, Method, StatsParams, VERSION};
use crate::{Client, Error};

/// How long `ssh` may take to connect, in seconds.
const CONNECT_TIMEOUT: u32 = 10;
/// Config writes so far, for their temp names.
static WRITES: AtomicU64 = AtomicU64::new(0);

/// A machine: a name and the `ssh` target it stands for.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Machine {
    /// The name `--on` takes.
    pub name: String,
    /// The `ssh` target, `[user@]host`.
    pub target: String,
}

fn bad_name(name: &str) -> bool {
    name.is_empty()
        || !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

fn bad_target(target: &str) -> bool {
    target.is_empty()
        || target.starts_with('-')
        || target
            .chars()
            .any(|c| c.is_whitespace() || c.is_control() || c == '"' || c == '\\')
}

/// The machines in the config file at `path`; none when it does not exist.
///
/// # Errors
/// [`Error::Config`] on anything but the `[machines]` table of `name = "target"` lines;
/// [`Error::Io`] when the file cannot be read.
pub fn load(path: &Path) -> Result<Vec<Machine>, Error> {
    let text = match fs::read_to_string(path) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        text => text?,
    };
    let mut machines = Vec::new();
    let mut in_machines = false;
    for (n, line) in text.lines().enumerate() {
        let line = line.trim();
        let fail = |what: &str| Error::Config(format!("{}:{}: {what}", path.display(), n + 1));
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(table) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            if table.trim() != "machines" {
                return Err(fail(&format!(
                    "unknown table [{table}]; shed knows [machines]"
                )));
            }
            in_machines = true;
            continue;
        }
        let Some((name, value)) = line.split_once('=') else {
            return Err(fail("expected name = \"ssh-target\""));
        };
        let (name, value) = (name.trim(), value.trim());
        if !in_machines {
            return Err(fail(&format!(
                "unknown key {name}; put machines under [machines]"
            )));
        }
        let target = value.strip_prefix('"').and_then(|v| v.strip_suffix('"'));
        match target {
            Some(target) if !bad_name(name) && !bad_target(target) => machines.push(Machine {
                name: name.to_owned(),
                target: target.to_owned(),
            }),
            _ => return Err(fail("expected name = \"ssh-target\"")),
        }
    }
    Ok(machines)
}

/// Writes `machines` to the config file at `path` (atomically).
///
/// # Errors
/// [`Error::Io`] when the file cannot be written.
pub fn save(path: &Path, machines: &[Machine]) -> Result<(), Error> {
    let mut text = String::from("# shed machines; edit with `shed machine add|rm`.\n[machines]\n");
    for m in machines {
        // Writing to a String cannot fail.
        let _ = writeln!(text, "{} = \"{}\"", m.name, m.target);
    }
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    // A temp name of its own per write, so parallel writers never share one.
    let n = WRITES.fetch_add(1, Ordering::Relaxed);
    let tmp = path.with_extension(format!("toml.{}.{n}.tmp", std::process::id()));
    fs::write(&tmp, text)?;
    fs::rename(&tmp, path)?;
    Ok(())
}

/// Adds machine `name` for `target` to the config file at `path`.
///
/// # Errors
/// [`Error::Config`] for a bad name or target, or a name already taken; [`load`]'s and [`save`]'s.
pub fn add(path: &Path, name: &str, target: &str) -> Result<Machine, Error> {
    if bad_name(name) {
        return Err(Error::Config(format!(
            "machine name {name:?}: use letters, digits, - and _"
        )));
    }
    if bad_target(target) {
        return Err(Error::Config(format!(
            "ssh target {target:?}: use [user@]host, no spaces or quotes"
        )));
    }
    let mut machines = load(path)?;
    if machines.iter().any(|m| m.name == name) {
        return Err(Error::Config(format!(
            "machine {name} exists; `shed machine rm {name}` first"
        )));
    }
    let machine = Machine {
        name: name.to_owned(),
        target: target.to_owned(),
    };
    machines.push(machine.clone());
    save(path, &machines)?;
    Ok(machine)
}

/// Removes machine `name` from the config file at `path`.
///
/// # Errors
/// [`Error::NoMachine`] when there is none; [`load`]'s and [`save`]'s.
pub fn remove(path: &Path, name: &str) -> Result<Machine, Error> {
    let mut machines = load(path)?;
    let at = machines
        .iter()
        .position(|m| m.name == name)
        .ok_or_else(|| Error::NoMachine(name.to_owned()))?;
    let machine = machines.remove(at);
    save(path, &machines)?;
    Ok(machine)
}

/// Machine `name` from the config file at `path`.
///
/// # Errors
/// [`Error::NoMachine`] when there is none; [`load`]'s.
pub fn find(path: &Path, name: &str) -> Result<Machine, Error> {
    load(path)?
        .into_iter()
        .find(|m| m.name == name)
        .ok_or_else(|| Error::NoMachine(name.to_owned()))
}

/// A client for the server on `machine`, through one `ssh <target> shed connect [--autostart]` with
/// key login only (`BatchMode=yes`), a connect timeout and a shared connection (`ControlMaster=auto`,
/// its socket in `control_dir`) where `ssh` supports it.
///
/// # Errors
/// [`Error::MachineVersion`] when the shed there is another version; [`Error::Unreachable`] when
/// `ssh` or the shed there gives no answer; [`Error::Io`] when `ssh` cannot start.
pub fn connect(machine: &Machine, control_dir: &Path, autostart: bool) -> Result<Client, Error> {
    let (ours, theirs) = UnixStream::pair()?;
    // Best effort: without the directory ssh warns and runs without a shared connection.
    let _ = fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(control_dir);
    let mut cmd = Command::new("ssh");
    for option in [
        "BatchMode=yes".to_owned(),
        format!("ConnectTimeout={CONNECT_TIMEOUT}"),
        "ControlMaster=auto".to_owned(),
        format!("ControlPath={}", control_dir.join("ssh-%C").display()),
        "ControlPersist=60".to_owned(),
    ] {
        cmd.arg("-o").arg(option);
    }
    cmd.args(["--", &machine.target, "shed", "connect"]);
    if autostart {
        cmd.arg("--autostart");
    }
    let mut child = cmd
        .stdin(Stdio::from(OwnedFd::from(theirs.try_clone()?)))
        .stdout(Stdio::from(OwnedFd::from(theirs)))
        .spawn()?;
    // Reaped when it ends (this side closing ends it), so a server's reconnects leave no zombies.
    std::thread::spawn(move || child.wait());
    // `cmd` keeps its copies of `theirs`; closing them lets a dead ssh show here as EOF.
    drop(cmd);
    match Client::over(ours) {
        Err(Error::Rpc(e)) if e.code == ErrorCode::Version.code() => {
            // The message is `SHED_VERSION: client x, server y; …` (proto::RpcError::version).
            let there = e.message.split("server ").nth(1).unwrap_or_default();
            let there = there.split(';').next().unwrap_or_default().to_owned();
            Err(Error::MachineVersion {
                machine: machine.name.clone(),
                here: VERSION.to_owned(),
                there,
            })
        }
        Err(Error::BadResponse(_) | Error::Io(_)) => Err(Error::Unreachable {
            machine: machine.name.clone(),
            target: machine.target.clone(),
        }),
        client => client,
    }
}

/// How long a machine has to answer `machine.stats` before it counts as offline.
pub const STATS_TIMEOUT: Duration = Duration::from_secs(5);

/// A machine that did not answer `machine.stats` within [`STATS_TIMEOUT`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Offline {
    /// The machine.
    pub machine: String,
    /// Always `false`.
    pub online: bool,
    /// When it last answered (unix seconds); `None` if never.
    pub last_seen: Option<u64>,
}

/// `machine.stats` on `machine` ([`connect`]), or [`Offline`] when it gives no answer within
/// [`STATS_TIMEOUT`], however `ssh` hangs. Each answer's time is kept in `seen_dir`.
///
/// # Errors
/// [`Error::MachineVersion`] when the shed there is another version.
pub fn stats(
    machine: &Machine,
    control_dir: &Path,
    seen_dir: &Path,
    params: StatsParams,
) -> Result<Result<MachineStats, Offline>, Error> {
    let (tx, rx) = mpsc::channel();
    let (m, dir) = (machine.clone(), control_dir.to_owned());
    std::thread::spawn(move || {
        let _ = tx
            .send(connect(&m, &dir, false).and_then(|mut c| c.call(Method::MachineStats, &params)));
    });
    let seen = seen_dir.join(&machine.name);
    match rx.recv_timeout(STATS_TIMEOUT) {
        Ok(Ok(stats)) => {
            // Best effort: without it a later offline answer says never seen.
            let _ = fs::create_dir_all(seen_dir);
            let _ = fs::write(&seen, crate::stats::unix_now().to_string());
            Ok(Ok(stats))
        }
        Ok(Err(e @ Error::MachineVersion { .. })) => Err(e),
        _ => Ok(Err(Offline {
            machine: machine.name.clone(),
            online: false,
            last_seen: fs::read_to_string(&seen)
                .ok()
                .and_then(|t| t.trim().parse().ok()),
        })),
    }
}

/// The wait before reconnecting after `failures` failed tries since a link dropped: 5 s for the first
/// five, then doubling up to 60 s.
#[must_use]
pub fn backoff(failures: u32) -> Duration {
    let secs = if failures < 5 {
        5
    } else {
        5 << (failures - 4).min(4)
    };
    Duration::from_secs(secs.min(60))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("shed-machines-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn add_ls_rm_round_trip_through_the_file() {
        let path = dir("round").join("config.toml");
        assert_eq!(load(&path).unwrap(), vec![]);
        add(&path, "box", "user@box.example").unwrap();
        add(&path, "b2", "b2.example").unwrap();
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.contains("[machines]\nbox = \"user@box.example\"\nb2 = \"b2.example\"\n"));
        assert_eq!(find(&path, "b2").unwrap().target, "b2.example");
        assert!(matches!(add(&path, "box", "x"), Err(Error::Config(_))));
        assert_eq!(remove(&path, "box").unwrap().target, "user@box.example");
        assert!(matches!(remove(&path, "box"), Err(Error::NoMachine(_))));
        assert_eq!(load(&path).unwrap().len(), 1);
    }

    /// R18: parallel writers each get their own temp name; every write lands and none is left behind.
    #[test]
    fn parallel_writes_never_share_a_temp_file() {
        let path = dir("parallel").join("config.toml");
        std::thread::scope(|s| {
            for t in 0..8 {
                let path = &path;
                s.spawn(move || {
                    for n in 0..50 {
                        let m = Machine {
                            name: format!("m{t}"),
                            target: format!("h{n}.example"),
                        };
                        save(path, &[m]).unwrap();
                    }
                });
            }
        });
        assert_eq!(load(&path).unwrap().len(), 1);
        let left: Vec<_> = fs::read_dir(path.parent().unwrap()).unwrap().collect();
        assert_eq!(left.len(), 1, "{left:?}");
    }

    /// R2: the config file the berth release wrote loads as it is.
    #[test]
    fn the_berth_release_config_loads() {
        let path = dir("berth").join("config.toml");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let old = "# berth machines; edit with `berth machine add|rm`.\n[machines]\nbox = \"user@box.example\"\n";
        fs::write(&path, old).unwrap();
        assert_eq!(find(&path, "box").unwrap().target, "user@box.example");
    }

    #[test]
    fn reconnects_follow_the_backoff_schedule() {
        let secs = |n| backoff(n).as_secs();
        assert_eq!(
            (0..10).map(secs).collect::<Vec<_>>(),
            [5, 5, 5, 5, 5, 10, 20, 40, 60, 60]
        );
    }

    #[test]
    fn bad_names_targets_and_files_are_refused() {
        let path = dir("bad").join("config.toml");
        for (name, target) in [
            ("", "h"),
            ("a b", "h"),
            ("a", ""),
            ("a", "-oProxyCommand=x"),
            ("a", "h\""),
            ("a", "h x"),
        ] {
            assert!(matches!(add(&path, name, target), Err(Error::Config(_))));
        }
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        for (text, hint) in [
            ("[other]\n", "unknown table [other]"),
            ("box = \"h\"\n", "unknown key box"),
            ("[machines]\nbox = h\n", ":2: expected"),
            ("[machines]\nbox\n", ":2: expected"),
        ] {
            fs::write(&path, text).unwrap();
            let e = load(&path).unwrap_err().to_string();
            assert!(e.contains(hint), "{e}");
        }
    }
}
