use super::{Entry, Registry, internal, load::history, warm::recipe};
use shed_core::{History, Size, Terminal};
use shed_proto::{ErrorCode, EventKind, RpcError, RunParams, State, TerminalInfo};
use std::fs;
use std::os::fd::AsFd as _;
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

pub(super) fn size(rows: Option<u16>, cols: Option<u16>) -> Size {
    let (r, c) = crate::config::default_size();
    Size {
        rows: rows.unwrap_or(r),
        cols: cols.unwrap_or(c),
    }
}

pub(super) fn spawn(
    size: Size,
    cmd: &[String],
    cwd: &Path,
    idle_after: Option<u64>,
    history: &History,
    env: &[(String, String)],
    acp: bool,
) -> Result<Terminal, shed_core::Error> {
    let env = env.iter().map(|(k, v)| (k, v));
    let pair = acp.then(UnixStream::pair);
    let (ours, theirs) = pair.transpose()?.unzip();
    let term = Terminal::spawn_with(cmd, cwd, env, size, history, theirs.map(Into::into))?;
    if let Some(ours) = ours {
        crate::handover::hand_over(ours, &[term.master()?.as_fd()], b"null")?;
    }
    term.set_idle_after(idle_after.map(Duration::from_secs));
    Ok(term)
}

fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

impl Registry {
    pub(crate) fn run(&mut self, mut p: RunParams) -> Result<TerminalInfo, RpcError> {
        let bad = |what: &str| RpcError::new(ErrorCode::InvalidParams, what);
        if p.warm && (p.acp.is_none() || p.resume.is_some() || !p.env.is_empty()) {
            return Err(bad("warm needs acp and takes no resume or env"));
        }
        if p.acp.is_none() && (p.model.is_some() || p.effort.is_some()) {
            return Err(bad("model and effort need acp"));
        }
        let recipe = p.warm.then(|| recipe(&p));
        if p.acp.is_some() {
            crate::drivers::launch::bridge(&mut p, &self.socket, &self.path, &self.providers)?;
        }
        if p.cmd.is_empty() {
            return Err(bad("empty cmd"));
        }
        if !p.cwd.is_absolute() {
            return Err(bad("cwd must be absolute"));
        }
        if p.rows == Some(0) || p.cols == Some(0) || p.mem_max_mb == Some(0) {
            return Err(bad("rows, cols and mem_max_mb must be at least 1"));
        }
        if p.labels.keys().any(|k| k.is_empty() || k.contains('=')) {
            return Err(bad("label keys must be non-empty and have no '='"));
        }
        crate::drivers::acp::mcp_servers(&p.mcp_servers).map_err(|e| bad(&e))?;
        if let Some(parent) = &mut p.parent {
            parent.name = self.named(&parent.name);
        }
        if let Some(parent) = p.parent.as_ref().map(|p| &p.name) {
            let gone = |_| {
                RpcError::new(
                    ErrorCode::NoSuchTerminal,
                    format!("parent {parent} is not running"),
                )
            };
            self.live(parent).map_err(gone)?;
        }
        let name = match p.name.take() {
            Some(name) if !valid_name(&name) => return Err(bad("name must match [A-Za-z0-9._-]+")),
            Some(name) if self.entries.contains_key(&name) => {
                return Err(RpcError::new(ErrorCode::NameTaken, name));
            }
            Some(name) => name,
            None => (1..=self.entries.len() + 1)
                .map(|n| format!("t{n}"))
                .find(|n| !self.entries.contains_key(n))
                .unwrap_or_default(),
        };
        if let Some(spare) = recipe.and_then(|r| self.spares().find(|e| e.ready(&r))) {
            let spare = spare.info.name.clone();
            return self.take(&spare, name, p.labels, p.parent);
        }
        let history = history(&self.path, &name);
        if let Some(log) = &history.log {
            fs::remove_file(log).ok();
        }
        let vars = crate::drivers::launch::vars(&p, &name).map_err(internal)?;
        let mem_max_mb = p.mem_max_mb.or_else(crate::config::mem_max);
        let scoped = mem_max_mb.filter(|_| crate::limit::scoped());
        let cmd = scoped.map(|mb| crate::limit::in_scope(&p.cmd, mb));
        let term = spawn(
            size(p.rows, p.cols),
            cmd.as_ref().unwrap_or(&p.cmd),
            &p.cwd,
            p.idle_after,
            &history,
            &vars,
            p.acp.is_some(),
        )
        .map_err(|e| RpcError::new(ErrorCode::SpawnFailed, format!("{name}: {e}")))?;
        if let Some(agent) = p.labels.get("agent").filter(|_| p.acp.is_none()) {
            term.set_rules(self.rules(agent));
        }
        let info = TerminalInfo {
            name: name.clone(),
            labels: p.labels,
            cwd: p.cwd,
            cmd: p.cmd,
            keep: p.keep,
            state: State::Running,
            exit: None,
            pid: Some(term.pid()),
            reported: None,
            records: Vec::new(),
            idle_after: p.idle_after,
            resources: None,
            acp: p.acp,
            session: None,
            parent: p.parent,
            mem_max_mb,
            redacted: false,
        };
        self.events.emit(&info, EventKind::Started);
        let entry = Entry {
            started: Some(shed_core::clock::now()),
            ..Entry::new(info.clone(), Some(Arc::new(term)))
        };
        self.entries.insert(name, entry);
        Ok(self.saved(info))
    }
}
