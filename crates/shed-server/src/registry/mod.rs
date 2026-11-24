use crate::events::Events;
use crate::input::Draft;
use crate::stats::Stats;
use shed_core::Terminal;
use shed_proto::{ErrorCode, LsParams, PROTOCOL, Resources, RpcError, State, TerminalInfo};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

mod acp;
mod exit;
mod family;
mod guard;
mod handover;
mod io;
mod load;
pub(crate) mod procs;
mod run;
mod save;
mod stream;
mod warm;

use guard::Guard;
use handover::Handed;
use load::history;
use save::Saver;
pub(crate) use save::{answering, keep_saved};
pub(crate) use stream::Stream;

pub(crate) const GRACE: Duration = Duration::from_secs(2);

struct Entry {
    info: TerminalInfo,
    term: Option<Arc<Terminal>>,
    draft: Arc<Draft>,
    exited_at: Option<u64>,
    killing: bool,
    resources: Option<Resources>,
    emitted: Option<Resources>,
    agent: Option<BTreeSet<i32>>,
    sampled: (Instant, Duration),
    notes: Vec<String>,
    started: Option<Instant>,
    prompted: Option<Instant>,
    recipe: Option<String>,
    was: Option<String>,
    speaks: Option<u32>,
    guard: Guard,
}

pub(crate) struct Registry {
    entries: BTreeMap<String, Entry>,
    pub(crate) path: PathBuf,
    pub(crate) providers: PathBuf,
    socket: PathBuf,
    pub(crate) saver: Saver,
    pub(crate) events: Events,
    pub(crate) stats: Stats,
    handed: Option<Handed>,
    pub(crate) control: Option<Arc<crate::handover::Control>>,
}

fn not_found(name: &str) -> RpcError {
    RpcError::new(ErrorCode::NoSuchTerminal, name)
}

fn internal(e: impl std::fmt::Display) -> RpcError {
    RpcError::new(ErrorCode::Internal, e)
}

impl Entry {
    fn new(info: TerminalInfo, term: Option<Arc<Terminal>>) -> Self {
        Self {
            term,
            draft: Arc::default(),
            exited_at: None,
            killing: false,
            resources: None,
            emitted: None,
            agent: None,
            sampled: (shed_core::clock::now(), Duration::ZERO),
            notes: Vec::new(),
            started: None,
            prompted: None,
            recipe: None,
            was: None,
            speaks: info.acp.is_some().then_some(PROTOCOL),
            guard: Guard::default(),
            info,
        }
    }

    fn view(&self) -> TerminalInfo {
        let mut info = TerminalInfo {
            resources: self.resources,
            ..self.info.clone()
        };
        if info.redacted {
            info.reported = info.reported.as_ref().and_then(crate::events::redacted);
            info.records = info
                .records
                .iter()
                .filter_map(crate::events::redacted)
                .collect();
        }
        let idle = ["done", "idle", "hibernated"];
        let reported = info.reported.as_ref().map(|r| r.state.as_str());
        if let Some(s) =
            (info.session.as_mut()).filter(|_| reported.is_some_and(|r| idle.contains(&r)))
        {
            let warm = s.cache_warm(shed_core::clock::unix_now());
            s.cache_likely = warm.map(|w| if w { "warm" } else { "cold" }.to_owned());
        }
        info
    }
}

impl Registry {
    fn entry(&self, name: &str) -> Result<&Entry, RpcError> {
        self.entries.get(name).ok_or_else(|| not_found(name))
    }

    pub(crate) fn live(&self, name: &str) -> Result<&Arc<Terminal>, RpcError> {
        let entry = self.entry(name)?;
        match (&entry.term, entry.info.state) {
            (Some(term), State::Running) => Ok(term),
            _ => Err(RpcError::new(
                ErrorCode::NoSuchTerminal,
                format!("{name} has exited"),
            )),
        }
    }

    pub(crate) fn named(&self, name: &str) -> String {
        let mut entries = self.entries.values();
        let taker = (!self.entries.contains_key(name))
            .then(|| entries.find(|e| e.was.as_deref() == Some(name)))
            .flatten();
        taker.map_or_else(|| name.to_owned(), |e| e.info.name.clone())
    }

    pub(crate) fn fresh(&self, name: &str) -> Option<Arc<Terminal>> {
        let entry = self.entries.get(name).filter(|e| e.was.is_none())?;
        entry
            .term
            .clone()
            .filter(|_| entry.info.state == State::Running)
    }

    fn files(&self, name: &str) -> [Option<PathBuf>; 2] {
        let lent = Some(crate::lane::pty_socket(&self.socket, name));
        [history(&self.path, name).log, lent]
    }

    pub(crate) fn ls(&self, p: &LsParams) -> Vec<TerminalInfo> {
        self.entries
            .values()
            .filter(|e| {
                p.labels
                    .iter()
                    .all(|(k, v)| e.info.labels.get(k) == Some(v))
            })
            .filter(|e| {
                let parent = e.info.parent.as_ref().map(|p| &p.name);
                p.parent.is_none() || parent == p.parent.as_ref()
            })
            .map(Entry::view)
            .collect()
    }

    pub(crate) fn get(&self, name: &str) -> Result<TerminalInfo, RpcError> {
        Ok(self.entry(name)?.view())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use shed_core::Terminal;

    use std::fs;

    use std::path::{Path, PathBuf};
    use std::sync::Arc;

    pub(super) fn dir(name: &str) -> PathBuf {
        crate::temp_dir(&format!("registry-{name}"))
    }

    pub(super) fn sleeper(registry: &mut Registry, name: &str) -> Arc<Terminal> {
        let run = serde_json::json!({"name": name, "cwd": "/", "cmd": ["sleep", "100"]});
        registry.run(serde_json::from_value(run).unwrap()).unwrap();
        Arc::clone(registry.live(name).unwrap())
    }

    pub(super) fn saved(path: &Path) -> serde_json::Value {
        serde_json::from_slice(&fs::read(path).unwrap()).unwrap()
    }
}
