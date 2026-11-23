//! Machines over SSH: the `[machines]` table of the config file and a [`Conn`] through
//! `ssh <target> shed connect`.
//!
//! The config file is a small TOML subset, written by `shed machine add|rm`:
//!
//! ```toml
//! [machines]
//! box = "user@box.example"
//! boat1 = { target = "ssh://user@vm.example:2222", host_key = "ssh-ed25519 AAAA…", identity = "~/.ssh/vm" }
//! ```
//!
//! Names are letters, digits, `-` and `_`; targets and identities are plain strings without quotes,
//! backslashes, commas, braces or spaces, a host key two of those. Anything else is an error that names the line.

use std::fmt::Write as _;
use std::fs;
use std::io;
use std::os::fd::OwnedFd;
use std::os::unix::fs::DirBuilderExt as _;
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::Serialize;
use serde_json::Value;

use crate::Error;
use crate::proto::{
    BUILD, ErrorCode, ExecParams, ExecResult, Link, MachineParams, MachineStats, Method, RpcError,
    StatsParams,
};
use shed_client::Conn;
use shed_core::runner::Child;

const CONNECT_TIMEOUT: u32 = 10;
const HELLO_TIMEOUT: Duration = Duration::from_secs(20);
static WRITES: AtomicU64 = AtomicU64::new(0);

pub use crate::proto::Machine;

pub(crate) fn entry(m: &Machine) -> String {
    if m.host_key.is_none() && m.identity.is_none() {
        return format!("\"{}\"", m.target);
    }
    let mut fields = vec![format!("target = \"{}\"", m.target)];
    fields.extend(m.host_key.iter().map(|k| format!("host_key = \"{k}\"")));
    let identity = m.identity.iter().map(|i| i.display());
    fields.extend(identity.map(|i| format!("identity = \"{i}\"")));
    format!("{{ {} }}", fields.join(", "))
}

pub(crate) fn key_changed(link: &Link) -> bool {
    (link.error.as_deref()).is_some_and(|e| e.starts_with("KEY_CHANGED"))
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
            .any(|c| c.is_whitespace() || c.is_control() || "\"\\,{}".contains(c))
}

fn check(m: &Machine) -> Result<(), Error> {
    let key = |k: &String| k.split(' ').count() != 2 || k.split(' ').any(bad_target);
    let identity = m.identity.as_deref().map(Path::to_string_lossy);
    let bad = if bad_name(&m.name) {
        format!("machine name {:?}: use letters, digits, - and _", m.name)
    } else if bad_target(&m.target) {
        format!(
            "ssh target {:?}: use [user@]host or ssh://[user@]host[:port]",
            m.target
        )
    } else if let Some(k) = m.host_key.as_ref().filter(|k| key(k)) {
        format!("host key {k:?}: use `<type> <base64>`")
    } else if let Some(i) = identity.filter(|i| bad_target(i)) {
        format!("identity {i:?}: a key file path, no spaces or quotes")
    } else {
        return Ok(());
    };
    Err(Error::Config(bad))
}

fn parse(name: &str, value: &str) -> Option<Machine> {
    let quoted = |v: &str| Some(v.trim().strip_prefix('"')?.strip_suffix('"')?.to_owned());
    let mut m = Machine {
        name: name.to_owned(),
        ..Machine::default()
    };
    let Some(fields) = value.strip_prefix('{').and_then(|v| v.strip_suffix('}')) else {
        m.target = quoted(value)?;
        return Some(m);
    };
    for field in fields.split(',') {
        let (key, v) = field.split_once('=')?;
        let v = quoted(v)?;
        match key.trim() {
            "target" => m.target = v,
            "host_key" => m.host_key = Some(v),
            "identity" => m.identity = Some(v.into()),
            _ => return None,
        }
    }
    Some(m)
}

/// The machines in the config file at `path`; none when it does not exist.
///
/// # Errors
/// [`Error::Config`] on anything but the `[machines]` table of `name = "target"` or `name = { target = … }` lines;
/// [`Error::Io`] when the file cannot be read.
pub fn load(path: &Path) -> Result<Vec<Machine>, Error> {
    let text = match fs::read_to_string(path) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        text => text?,
    };
    let mut machines = Vec::new();
    let mut table = None;
    for (n, line) in text.lines().enumerate() {
        let line = line.trim();
        let fail = |what: &str| Error::Config(format!("{}:{}: {what}", path.display(), n + 1));
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            if !["machines", "script"].contains(&name.trim()) {
                return Err(fail(&format!(
                    "unknown table [{name}]; shed knows [machines] and [script]"
                )));
            }
            table = Some(name.trim() == "machines");
            continue;
        }
        if table == Some(false) {
            continue;
        }
        let Some((name, value)) = line.split_once('=') else {
            return Err(fail("expected name = \"ssh-target\""));
        };
        let (name, value) = (name.trim(), value.trim());
        if table.is_none() {
            return Err(fail(&format!(
                "unknown key {name}; put machines under [machines]"
            )));
        }
        match parse(name, value).filter(|m| check(m).is_ok()) {
            Some(machine) => machines.push(machine),
            None => {
                return Err(fail(
                    "expected name = \"ssh-target\" or name = { target = \"…\" }",
                ));
            }
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
        writeln!(text, "{} = {}", m.name, entry(m)).ok();
    }
    let old = fs::read_to_string(path).unwrap_or_default();
    let mut script = false;
    for line in old.lines() {
        if let Some(name) = line
            .trim()
            .strip_prefix('[')
            .and_then(|l| l.strip_suffix(']'))
        {
            script = name.trim() == "script";
        }
        if script {
            writeln!(text, "{line}").ok();
        }
    }
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let n = WRITES.fetch_add(1, Ordering::Relaxed);
    let tmp = path.with_extension(format!("toml.{}.{n}.tmp", shed_core::runner::pid()));
    fs::write(&tmp, text)?;
    fs::rename(&tmp, path)?;
    Ok(())
}

/// Adds machine `p.name` to the config file at `path`; the same values again change nothing, other values
/// replace its line only with `p.replace`.
///
/// # Errors
/// [`Error::Config`] for a bad name, target, key or identity, or a name taken by other values without
/// `replace`; [`load`]'s and [`save`]'s.
pub fn add(path: &Path, p: &MachineParams) -> Result<Machine, Error> {
    let machine = p.machine.clone();
    check(&machine)?;
    let mut machines = load(path)?;
    match machines.iter_mut().find(|m| m.name == machine.name) {
        Some(m) if *m == machine => return Ok(machine),
        Some(m) if p.replace => m.clone_from(&machine),
        Some(_) => {
            return Err(Error::Config(format!(
                "machine {} exists with other values; `--replace` moves it",
                machine.name
            )));
        }
        None => machines.push(machine.clone()),
    }
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
pub fn connect(machine: &Machine, control_dir: &Path, autostart: bool) -> Result<Conn, Error> {
    Ok(line(machine, control_dir, autostart)?.0)
}

pub(crate) fn line(
    machine: &Machine,
    control_dir: &Path,
    autostart: bool,
) -> Result<(Conn, UnixStream), Error> {
    let (conn, ssh, line) = dial(machine, control_dir, autostart)?;
    ssh.keep();
    Ok((conn, line))
}

struct Ssh(Option<Child>);

impl Ssh {
    fn keep(mut self) {
        if let Some(mut child) = self.0.take() {
            std::thread::spawn(move || child.wait());
        }
    }
}

impl Drop for Ssh {
    fn drop(&mut self) {
        if let Some(mut child) = self.0.take() {
            child.kill().ok();
            std::thread::spawn(move || child.wait());
        }
    }
}

pub(crate) struct Ends(Arc<Mutex<(bool, Option<Ssh>)>>);

impl Drop for Ends {
    fn drop(&mut self) {
        let mut slot = crate::lock(&self.0);
        slot.0 = true;
        slot.1.take();
    }
}

pub(crate) fn alone(
    machine: &Machine,
    control_dir: &Path,
) -> (
    impl FnOnce(Method, &Value) -> Result<Value, Error> + Send + 'static,
    Ends,
) {
    let (machine, control_dir) = (machine.clone(), control_dir.to_owned());
    let ending = Arc::new(Mutex::new((false, None)));
    let ends = Ends(Arc::clone(&ending));
    let call = move |method, params: &Value| {
        let (mut conn, ssh, _) = dial(&machine, &control_dir, false)?;
        let mut slot = crate::lock(&ending);
        if slot.0 {
            return Err(io::Error::from(io::ErrorKind::Interrupted).into());
        }
        slot.1 = Some(ssh);
        drop(slot);
        Ok(conn.call(method, params)?)
    };
    (call, ends)
}

fn dial(
    machine: &Machine,
    control_dir: &Path,
    autostart: bool,
) -> Result<(Conn, Ssh, UnixStream), Error> {
    let (ours, theirs) = UnixStream::pair()?;
    let line = ours.try_clone()?;
    line.set_read_timeout(Some(HELLO_TIMEOUT))?;
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(control_dir)
        .ok();
    let options = [
        "BatchMode=yes".to_owned(),
        format!("ConnectTimeout={CONNECT_TIMEOUT}"),
        "ControlMaster=auto".to_owned(),
        format!("ControlPath={}", control_dir.join("ssh-%C").display()),
        "ControlPersist=60".to_owned(),
    ];
    let mut args = ssh_options(machine, control_dir, options.into());
    let log = control_dir.join(format!("{}.ssh.log", machine.name));
    if machine.host_key.is_some() && fs::write(&log, "").is_ok() {
        args.extend(["-E".to_owned(), log.display().to_string()]);
    }
    args.extend(["--", &machine.target, "shed", "connect"].map(str::to_owned));
    if autostart {
        args.push("--autostart".to_owned());
    }
    let ssh = Ssh(Some(shed_core::runner::over(
        "ssh",
        args,
        OwnedFd::from(theirs),
    )?));
    match Conn::over(ours) {
        Err(shed_client::Error::Version {
            theirs, refusal, ..
        }) => {
            let there = match (theirs, refusal) {
                (Some(theirs), _) => theirs.label(),
                (None, e) => {
                    let message = e.map(|e| e.message).unwrap_or_default();
                    let there = message.split(" / server ").nth(1).unwrap_or_default();
                    let there = there.split(" protocol ").next().unwrap_or_default();
                    there.to_owned()
                }
            };
            Err(Error::MachineVersion {
                machine: machine.name.clone(),
                here: BUILD.to_owned(),
                there,
            })
        }
        Err(shed_client::Error::Bad(_) | shed_client::Error::Io { .. })
            if machine.host_key.is_some()
                && fs::read_to_string(&log)
                    .is_ok_and(|t| t.contains("Host key verification failed")) =>
        {
            Err(Error::KeyChanged {
                machine: machine.name.clone(),
            })
        }
        Err(shed_client::Error::Bad(_) | shed_client::Error::Io { .. }) => {
            Err(Error::Unreachable {
                machine: machine.name.clone(),
                target: machine.target.clone(),
            })
        }
        conn => {
            let conn = conn?;
            line.set_read_timeout(None)?;
            Ok((conn, ssh, line))
        }
    }
}

pub(crate) fn ssh_options(
    machine: &Machine,
    control_dir: &Path,
    mut options: Vec<String>,
) -> Vec<String> {
    if let Some(key) = &machine.host_key {
        let known = control_dir.join(format!("{}.known_hosts", machine.name));
        fs::write(&known, format!("{} {key}\n", machine.name)).ok();
        options.extend([
            format!("HostKeyAlias={}", machine.name),
            format!("UserKnownHostsFile={}", known.display()),
            "StrictHostKeyChecking=yes".to_owned(),
        ]);
    }
    if let Some(identity) = &machine.identity {
        options.extend([
            format!("IdentityFile={}", identity.display()),
            "IdentitiesOnly=yes".to_owned(),
        ]);
    }
    options
        .into_iter()
        .flat_map(|o| ["-o".to_owned(), o])
        .collect()
}

pub(crate) fn facts(calls: &mut Conn) -> [Option<String>; 3] {
    let script = "uname -s; uname -m; printf '%s\\n' \"$HOME\"";
    let params = ExecParams {
        cmd: ["sh", "-c", script].map(str::to_owned).into(),
        timeout: Some(10.0),
        ..ExecParams::default()
    };
    let ran = calls.call::<_, ExecResult>(Method::Exec, &params);
    let out = ran.map(|r| String::from_utf8_lossy(&r.stdout).into_owned());
    let out = out.unwrap_or_default();
    let mut lines = out.lines().map(str::to_owned);
    let mut next = || lines.next().filter(|l| !l.is_empty());
    [next().map(|os| os.to_lowercase()), next(), next()]
}

pub(crate) fn refusal(e: Error) -> RpcError {
    match e {
        Error::Io(e) => RpcError::new(ErrorCode::Io, e),
        e => RpcError::new(ErrorCode::InvalidParams, e),
    }
}

const CALL_BOUND: Duration = Duration::from_secs(10);

pub(crate) fn bound(method: Method, params: &Value) -> Option<Duration> {
    let timeout = params.get("timeout").and_then(Value::as_f64);
    let timeout = timeout.and_then(|t| Duration::try_from_secs_f64(t).ok());
    match method {
        Method::Wait
        | Method::Script
        | Method::Run
        | Method::Send
        | Method::FileRead
        | Method::FileWrite => None,
        Method::Exec => timeout.map(|t| t + CALL_BOUND),
        _ => Some(CALL_BOUND),
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
        tx.send(
            connect(&m, &dir, false).and_then(|mut c| Ok(c.call(Method::MachineStats, &params)?)),
        )
        .ok();
    });
    let seen = seen_dir.join(&machine.name);
    match rx.recv_timeout(STATS_TIMEOUT) {
        Ok(Ok(stats)) => {
            fs::create_dir_all(seen_dir).ok();
            fs::write(&seen, shed_core::clock::unix_now().to_string()).ok();
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
        crate::temp_dir(&format!("machines-{name}"))
    }

    fn plain(name: &str, target: &str) -> MachineParams {
        let machine = Machine {
            name: name.into(),
            target: target.into(),
            ..Machine::default()
        };
        MachineParams {
            machine,
            replace: false,
        }
    }

    fn keyed(p: MachineParams, key: &str) -> MachineParams {
        let host_key = Some(key.to_owned());
        let machine = Machine {
            host_key,
            ..p.machine
        };
        MachineParams { machine, ..p }
    }

    #[test]
    fn a_pinned_machine_is_a_table_line_and_moves_only_with_replace() {
        let path = dir("pinned").join("config.toml");
        let mut pinned = keyed(
            plain("boat1", "ssh://user@vm.example:2222"),
            "ssh-ed25519 AAAAold",
        );
        pinned.machine.identity = Some("~/.ssh/vm".into());
        add(&path, &plain("box", "user@box.example")).unwrap();
        add(&path, &pinned).unwrap();
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.contains("box = \"user@box.example\"\nboat1 = { target = \"ssh://user@vm.example:2222\", host_key = \"ssh-ed25519 AAAAold\", identity = \"~/.ssh/vm\" }\n"), "{text}");
        let boat = find(&path, "boat1").unwrap();
        assert_eq!(boat.host_key.as_deref(), Some("ssh-ed25519 AAAAold"));
        assert_eq!(boat.identity, Some(std::path::PathBuf::from("~/.ssh/vm")));
        add(&path, &pinned).unwrap();
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            text,
            "the same values twice are one"
        );
        let rekeyed = keyed(pinned.clone(), "ssh-ed25519 AAAAnew");
        assert!(matches!(add(&path, &rekeyed), Err(Error::Config(_))));
        assert_eq!(fs::read_to_string(&path).unwrap(), text);
        add(
            &path,
            &MachineParams {
                replace: true,
                ..rekeyed
            },
        )
        .unwrap();
        let boat = find(&path, "boat1").unwrap();
        assert_eq!(boat.host_key.as_deref(), Some("ssh-ed25519 AAAAnew"));
        assert_eq!(load(&path).unwrap().len(), 2);
    }

    #[test]
    fn the_script_table_is_skipped_and_kept() {
        let path = dir("script").join("config.toml");
        let script = "[script]\nguard = [\n  \"drover\", \"guard\",\n]\nguard_timeout = 5\n";
        fs::write(
            &path,
            format!("[machines]\nbox = \"box.example\"\n{script}"),
        )
        .unwrap();
        add(&path, &plain("b2", "b2.example")).unwrap();
        let names: Vec<String> = load(&path).unwrap().into_iter().map(|m| m.name).collect();
        assert_eq!(names, ["box", "b2"]);
        assert!(fs::read_to_string(&path).unwrap().ends_with(script));
    }

    #[test]
    fn add_ls_rm_round_trip_through_the_file() {
        let path = dir("round").join("config.toml");
        assert_eq!(load(&path).unwrap(), vec![]);
        add(&path, &plain("box", "user@box.example")).unwrap();
        add(&path, &plain("b2", "b2.example")).unwrap();
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.contains("[machines]\nbox = \"user@box.example\"\nb2 = \"b2.example\"\n"));
        assert_eq!(find(&path, "b2").unwrap().target, "b2.example");
        assert!(matches!(
            add(&path, &plain("box", "x")),
            Err(Error::Config(_))
        ));
        add(&path, &plain("box", "user@box.example")).unwrap();
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            text,
            "the same pair twice is one"
        );
        assert_eq!(remove(&path, "box").unwrap().target, "user@box.example");
        assert!(matches!(remove(&path, "box"), Err(Error::NoMachine(_))));
        assert_eq!(load(&path).unwrap().len(), 1);
    }

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
                            ..Machine::default()
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
            assert!(matches!(
                add(&path, &plain(name, target)),
                Err(Error::Config(_))
            ));
        }
        for key in ["ssh-ed25519", "ssh-ed25519 AAAA x", "ssh-ed25519 AA,AA"] {
            let p = keyed(plain("a", "h"), key);
            assert!(matches!(add(&path, &p), Err(Error::Config(_))), "{key}");
        }
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        for (text, hint) in [
            ("[other]\n", "unknown table [other]"),
            ("box = \"h\"\n", "unknown key box"),
            ("[machines]\nbox = h\n", ":2: expected"),
            ("[machines]\nbox\n", ":2: expected"),
            (
                "[machines]\nbox = { target = \"h\", port = \"2\" }\n",
                ":2: expected",
            ),
            (
                "[machines]\nbox = { host_key = \"ssh-ed25519 AAAA\" }\n",
                ":2: expected",
            ),
        ] {
            fs::write(&path, text).unwrap();
            let e = load(&path).unwrap_err().to_string();
            assert!(e.contains(hint), "{e}");
        }
    }
}
