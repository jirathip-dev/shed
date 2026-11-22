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
    pub(crate) fn set_rules(&self) {
        for e in self.entries.values().filter(|e| e.info.acp.is_none()) {
            if let (Some(term), Some(agent)) = (&e.term, e.info.labels.get("agent")) {
                term.set_rules(self.rules(agent));
            }
        }
    }

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

    fn rules(&self, agent: &str) -> Vec<shed_core::Rule> {
        let catalog = crate::providers::Catalog::load(&self.providers).0;
        let rules = catalog
            .by_agent(agent)
            .map(|p| p.rules.clone())
            .unwrap_or_default();
        let rule = |r: crate::proto::Rule| shed_core::Rule {
            id: r.id,
            state: r.state,
            kind: r.kind,
            priority: r.priority,
            region: r.region,
            any: r.any,
        };
        rules.into_iter().map(rule).collect()
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
    use std::time::Duration;

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

    #[tokio::test]
    async fn each_shipped_rule_matches_its_golden_screen_and_unlabelled_terminals_never_check() {
        let path = dir("rules").join("state.json");
        let mut reg = Registry::load(&path, Path::new("shed.sock"), Vec::new()).unwrap();
        let screens = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/screens");
        let catalog = crate::providers::Catalog::load(&reg.providers).0;
        let rules = (catalog.iter().map(|e| &e.provider))
            .flat_map(|p| p.rules.iter().map(|r| (&p.agent, r)));
        let mut shipped = 0;
        for (n, (agent, rule)) in rules.enumerate() {
            assert!(!rule.version.is_empty(), "{}", rule.id);
            let golden = screens.join(format!("{agent}-{}.txt", rule.id));
            assert!(golden.exists(), "{}", golden.display());
            let mut run = |name: &str, labels: serde_json::Value| {
                let golden = golden.to_str().unwrap();
                let p = serde_json::json!({
                    "name": name, "cwd": "/", "labels": labels,
                    "cmd": ["sh", "-c", "cat \"$0\"; exec sleep 60", golden],
                });
                reg.run(serde_json::from_value(p).unwrap()).unwrap();
                Arc::clone(reg.live(name).unwrap())
            };
            let lane = run(&format!("lane{n}"), serde_json::json!({ "agent": agent }));
            let plain = run(&format!("plain{n}"), serde_json::json!({}));
            let deadline = shed_core::clock::now() + Duration::from_secs(10);
            let from = Some(format!("rule:{}", rule.id));
            let matched =
                lane.wait_until(|t| t.report().and_then(|r| r.from) == from, Some(deadline));
            assert_eq!(
                matched.await.unwrap(),
                shed_core::Wake::Matched,
                "{}",
                golden.display()
            );
            assert_eq!(lane.report().map(|r| r.state), Some(rule.state.clone()));
            let shown = plain.wait_until(|t| t.screen().contains(&rule.any[0]), Some(deadline));
            assert_eq!(shown.await.unwrap(), shed_core::Wake::Matched);
            assert_eq!(plain.report(), None);
            assert!(lane.rule_checks() > 0);
            assert_eq!(plain.rule_checks(), 0);
            for t in [lane, plain] {
                t.kill(Duration::ZERO).unwrap();
            }
            shipped += 1;
        }
        assert!(shipped > 0);
    }
}
