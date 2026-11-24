//! A command inside `srt`: the rules file shed writes on the machine that runs it, links resolved there, and the wrap.

use std::fs;
use std::io::Write as _;
use std::os::unix::fs::{DirBuilderExt as _, OpenOptionsExt as _};
use std::path::{Path, PathBuf};

use serde_json::json;
use shed_proto::{ErrorCode, RpcError, Sandbox, SandboxNeeds};

pub(crate) const WRAP_VAR: &str = "SHED_SANDBOX";

pub(crate) struct Own<'a> {
    pub(crate) state: &'a Path,
    pub(crate) providers: &'a Path,
    pub(crate) socket: &'a Path,
}

#[derive(Debug, Default)]
pub(crate) struct Wrap {
    pub(crate) prefix: Vec<String>,
    pub(crate) env: Vec<(String, String)>,
}

impl Wrap {
    pub(crate) fn around(&self, cmd: &[String]) -> Vec<String> {
        self.prefix.iter().chain(cmd).cloned().collect()
    }
}

fn bad(what: impl std::fmt::Display) -> RpcError {
    RpcError::new(ErrorCode::InvalidParams, what)
}

fn on_path(program: &str) -> Option<PathBuf> {
    let path = crate::config::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|d| d.join(program))
        .find(|p| p.is_file())
}

fn real(p: &Path) -> PathBuf {
    fs::canonicalize(p).unwrap_or_else(|_| p.to_owned())
}

pub(crate) fn file(state: &Path, name: &str) -> PathBuf {
    state.with_file_name("sandbox").join(format!("{name}.json"))
}

pub(crate) fn remove(state: &Path, name: &str) {
    let rules = file(state, name);
    fs::remove_file(&rules).ok();
    fs::remove_dir_all(rules.with_extension("tmp")).ok();
}

pub(crate) fn prepare(
    own: &Own<'_>,
    name: &str,
    sandbox: &Sandbox,
    (needs, config): (Option<&SandboxNeeds>, Option<&Path>),
) -> Result<Wrap, RpcError> {
    let srt = on_path("srt");
    let home = crate::accounts::home();
    let at = |s: &String| match s.strip_prefix("~/") {
        Some(rest) => home.join(rest),
        None if s == "{config}" => config.map_or_else(|| home.join(".config"), Path::to_path_buf),
        None => PathBuf::from(s),
    };
    let state = own.state.parent().unwrap_or(own.state);
    let configs = own.providers.parent().unwrap_or(own.providers);
    let exe = std::env::current_exe().map_err(|e| RpcError::new(ErrorCode::Internal, e))?;
    let installed = [srt.as_deref(), Some(&exe)].map(|p| Some(real(p?).parent()?.to_path_buf()));
    for w in &sandbox.write {
        let w = real(w);
        let ours = [state, configs].iter().any(|o| {
            let o = real(o);
            o.starts_with(&w) || w.starts_with(&o)
        });
        if !w.is_absolute() || ours || installed.iter().flatten().any(|i| i.starts_with(&w)) {
            return Err(bad(format!("sandbox may not write {}", w.display())));
        }
    }
    let srt = srt.ok_or_else(|| {
        RpcError::new(
            ErrorCode::SandboxMissing,
            "no srt on this machine's PATH; nothing ran",
        )
    })?;
    let rules = file(own.state, name);
    let tmp = sandbox
        .tmp
        .clone()
        .unwrap_or_else(|| rules.with_extension("tmp"));
    let made = |d: &Path| fs::DirBuilder::new().recursive(true).mode(0o700).create(d);
    made(&tmp).map_err(crate::call::io_error)?;
    let needs = needs.cloned().unwrap_or_default();
    let mut write = sandbox.write.clone();
    write.extend(needs.write.iter().map(at).chain([tmp.clone()]));
    let inside = crate::inside::socket(own.state);
    let mut read = sandbox.read.clone();
    read.extend(inside.parent().map(Path::to_path_buf));
    read.extend(needs.read.iter().map(at).chain(write.clone()));
    read.extend(installed.into_iter().flatten());
    let mut deny = vec![
        home.clone(),
        home.join(".ssh"),
        state.to_owned(),
        configs.to_owned(),
    ];
    deny.extend(
        [own.socket.to_owned()]
            .into_iter()
            .chain(sandbox.deny.iter().cloned()),
    );
    let sockets: Vec<PathBuf> = sandbox
        .sockets
        .iter()
        .cloned()
        .chain([inside.clone()])
        .collect();
    let hosts: Vec<&String> = sandbox.hosts.iter().chain(&needs.hosts).collect();
    let settings = json!({
        "filesystem": {
            "denyRead": resolved(deny, false),
            "allowRead": resolved(read, true),
            "allowWrite": resolved(write, true),
            "denyWrite": [],
        },
        "network": {
            "allowedDomains": hosts,
            "deniedDomains": [],
            "allowMachLookup": sandbox.mach,
            "allowUnixSockets": resolved(sockets, true),
            "allowAllUnixSockets": cfg!(target_os = "linux"),
        },
    });
    made(rules.parent().unwrap_or(state)).map_err(crate::call::io_error)?;
    let mut out = fs::OpenOptions::new();
    let out = out.write(true).create(true).truncate(true).mode(0o600);
    let written = out
        .open(&rules)
        .and_then(|mut f| f.write_all(settings.to_string().as_bytes()));
    written.map_err(crate::call::io_error)?;
    let path = |p: &Path| p.to_string_lossy().into_owned();
    let mut env = vec![("SHED_SOCKET".to_owned(), path(&inside))];
    env.extend(needs.tmp_var.map(|v| (v, path(&tmp))));
    Ok(Wrap {
        prefix: vec![path(&srt), "--settings".to_owned(), path(&rules)],
        env,
    })
}

fn resolved(paths: Vec<PathBuf>, links: bool) -> Vec<PathBuf> {
    let inner = |d: &PathBuf| {
        let entries = fs::read_dir(d)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path());
        entries
            .filter(|e| links && e.is_symlink())
            .collect::<Vec<_>>()
    };
    let hops = |p: &PathBuf| {
        let up = p.ancestors().skip(1).filter(|a| links && a.is_symlink());
        up.map(Path::to_path_buf).collect::<Vec<_>>()
    };
    let targets: Vec<PathBuf> = paths
        .iter()
        .flat_map(|p| std::iter::once(p.clone()).chain(inner(p)))
        .filter_map(|p| fs::canonicalize(p).ok())
        .collect();
    let up: Vec<PathBuf> = paths.iter().flat_map(hops).collect();
    let mut all: Vec<PathBuf> = paths.into_iter().chain(targets).chain(up).collect();
    all.sort();
    all.dedup();
    all
}
