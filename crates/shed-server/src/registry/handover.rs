use super::{
    Registry, load::adopt, load::history, save::Saved, save::StateFile, save::write_state,
};
use crate::handover::Control;
use serde::{Deserialize, Serialize};
use shed_core::{Detached, Report, Terminal};
use shed_proto::{State, TerminalInfo};
use std::collections::BTreeMap;
use std::io;
use std::os::fd::OwnedFd;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

type Frozen = Vec<(String, Detached)>;

const FIRST_LENDING_PROTOCOL: u32 = 1;

#[derive(Clone, Serialize, Deserialize)]
pub(super) struct Handed {
    pub(super) format: u32,
    pub(super) seq: u64,
    pub(super) screens: Vec<(String, String)>,
}

fn inherit(state: &Path, info: &TerminalInfo, master: OwnedFd, repaint: &str) -> Option<Terminal> {
    let history = history(state, &info.name);
    let term = Terminal::adopt(master, info.pid?, &history, Some(repaint)).ok()?;
    term.set_idle_after(info.idle_after.map(Duration::from_secs));
    let reported = info
        .reported
        .iter()
        .filter(|r| r.from.as_deref() == Some("reported"));
    for r in reported.chain(&info.records).cloned() {
        let (state, note, id, kind, progress) = (r.state, r.note, r.id, r.kind, r.progress);
        let (source, app, title, msg) = (r.state_source, r.app, r.title, r.msg);
        let report = Report {
            state,
            note,
            source,
            id,
            kind,
            progress,
            app,
            title,
            msg,
            from: None,
        };
        term.set_report(report);
    }
    Some(term)
}

pub(super) fn picked_up(
    path: &Path,
    socket: &Path,
    file: &StateFile,
    handed: Option<&Handed>,
    masters: Vec<OwnedFd>,
) -> BTreeMap<String, Terminal> {
    let running = |name: &str| {
        let running = |s: &&Saved| s.info.state == State::Running && s.info.name == name;
        file.terminals.iter().find(running).map(|s| &s.info)
    };
    let screens = handed.iter().flat_map(|h| h.screens.iter()).zip(masters);
    let mut adopted: BTreeMap<_, _> = (screens)
        .filter_map(|((name, repaint), master)| {
            Some((
                name.clone(),
                inherit(path, running(name)?, master, repaint)?,
            ))
        })
        .collect();
    let lanes: Vec<_> = (file.terminals.iter())
        .filter(|s| s.info.state == State::Running && s.info.acp.is_some())
        .filter(|s| !adopted.contains_key(&s.info.name))
        .filter_map(|s| Some((s.info.name.clone(), adopt(socket, path, &s.info)?)))
        .collect();
    adopted.extend(lanes);
    adopted
}

impl Registry {
    pub(crate) fn bridges(&self) -> impl Iterator<Item = (&str, u32)> {
        let lanes = self.entries.values().filter(|e| e.info.acp.is_some());
        let live = lanes.filter(|e| e.info.state == State::Running);
        live.map(|e| {
            (
                e.info.name.as_str(),
                e.speaks.unwrap_or(FIRST_LENDING_PROTOCOL),
            )
        })
    }

    pub(crate) fn freeze(&mut self) -> (Frozen, io::Result<()>, Option<Arc<Control>>) {
        let running = self.running().into_iter();
        let detached = running.filter_map(|(name, term)| Some((name, term.detach().ok()?)));
        let mut frozen: Vec<_> = detached.collect();
        self.refresh().ok();
        frozen.retain(|(name, _)| self.live(name).is_ok());
        self.handed = Some(Handed {
            format: crate::handover::FORMAT,
            seq: self.events.seq(),
            screens: (frozen.iter().map(|(n, d)| (n.clone(), d.repaint.clone()))).collect(),
        });
        let saved = write_state(&self.path, &self.state_file());
        (frozen, saved, self.control.clone())
    }

    pub(crate) fn thaw(&mut self, frozen: Frozen) -> Vec<(String, Arc<Terminal>)> {
        self.handed = None;
        let mut back = Vec::new();
        for (name, Detached { master, repaint }) in frozen {
            let Some(entry) = self.entries.get_mut(&name) else {
                continue;
            };
            if let Some(term) = inherit(&self.path, &entry.info, master, &repaint).map(Arc::new) {
                entry.term = Some(Arc::clone(&term));
                back.push((name, term));
            }
        }
        self.set_rules();
        self.saved(back)
    }
}
