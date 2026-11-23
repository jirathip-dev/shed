use super::{Entry, Registry, not_found};
use shed_proto::{EventKind, Parent, RpcError, RunParams, State, TerminalInfo};
use std::collections::BTreeMap;
use std::fs;

pub(super) fn recipe(p: &RunParams) -> String {
    let (name, labels, parent, warm) = (None, BTreeMap::new(), None, false);
    let recipe = RunParams {
        name,
        labels,
        parent,
        warm,
        ..p.clone()
    };
    serde_json::to_string(&recipe).unwrap_or_default()
}

impl Entry {
    pub(super) fn ready(&self, recipe: &str) -> bool {
        self.recipe.as_deref() == Some(recipe) && self.info.session.is_some()
    }
}

impl Registry {
    pub(super) fn take(
        &mut self,
        spare: &str,
        name: String,
        labels: BTreeMap<String, String>,
        parent: Option<Parent>,
    ) -> Result<TerminalInfo, RpcError> {
        let mut entry = self.entries.remove(spare).ok_or_else(|| not_found(spare))?;
        for (from, to) in self.files(spare).into_iter().zip(self.files(&name)) {
            if let (Some(from), Some(to)) = (from, to) {
                fs::rename(from, to).ok();
            }
        }
        (entry.info.name, entry.info.labels, entry.info.parent) = (name.clone(), labels, parent);
        (entry.recipe, entry.was) = (None, Some(spare.to_owned()));
        if let Some(session) = &mut entry.info.session {
            session.ready_ms = Some(0);
        }
        if let Some(term) = entry.term.clone() {
            let text = shed_proto::frame(&shed_proto::Input::Name(name.clone()));
            std::thread::spawn(move || term.write(text.as_bytes()).ok());
        }
        self.events.emit(&entry.info, EventKind::Started);
        let info = entry.info.clone();
        self.entries.insert(name, entry);
        Ok(self.saved(info))
    }

    pub(super) fn spares(&self) -> impl Iterator<Item = &Entry> {
        let running = |e: &&Entry| e.info.state == State::Running && !e.killing;
        self.entries
            .values()
            .filter(|e| e.recipe.is_some())
            .filter(running)
    }

    pub(crate) fn warm_up(&mut self, p: RunParams) -> Option<String> {
        let recipe = recipe(&p);
        let same = self.spares().any(|e| e.recipe.as_ref() == Some(&recipe));
        if same || self.spares().count() >= crate::config::warm_max() {
            return None;
        }
        let free = |n: &String| !self.entries.contains_key(n) && self.named(n) == *n;
        let mut names = (1..=2 * self.entries.len() + 1).map(|n| format!(".warm-{n}"));
        let name = names.find(free)?;
        let labels = BTreeMap::new();
        let (parent, warm) = (None, false);
        let p = RunParams {
            name: Some(name.clone()),
            labels,
            parent,
            warm,
            ..p
        };
        self.run(p).ok()?;
        self.entries.get_mut(&name)?.recipe = Some(recipe);
        Some(name)
    }

    pub(crate) fn stale_spares(&self, idle: std::time::Duration) -> Vec<String> {
        let now = shed_core::clock::now();
        let stale = |e: &&Entry| {
            e.started
                .is_none_or(|t| now.saturating_duration_since(t) >= idle)
        };
        self.spares()
            .filter(stale)
            .map(|e| e.info.name.clone())
            .collect()
    }
}
