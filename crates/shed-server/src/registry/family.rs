use super::Registry;
use shed_core::Report;
use shed_proto::{State, TerminalInfo};
use std::sync::Arc;

pub(super) fn note(child: &TerminalInfo, what: &str) -> Option<(String, String)> {
    let parent = child.parent.as_ref().filter(|p| p.notify)?;
    Some((parent.name.clone(), format!("[shed] {} {what}", child.name)))
}

fn working(t: &TerminalInfo) -> bool {
    t.reported.as_ref().is_some_and(|r| r.state == "working")
}

impl Registry {
    pub(super) fn deliver(&mut self, notes: Vec<(String, String)>) {
        for (parent, note) in notes {
            if let Some(entry) = self.entries.get_mut(&parent) {
                entry.notes.push(note);
            }
        }
        for entry in self.entries.values_mut() {
            let Some(term) = &entry.term else { continue };
            if entry.notes.is_empty() || entry.info.state == State::Exited || working(&entry.info) {
                continue;
            }
            let notes = std::mem::take(&mut entry.notes);
            let text = if entry.info.acp.is_some() {
                let state = "working".to_owned();
                term.set_report(Report {
                    state,
                    ..Report::default()
                });
                shed_proto::frame(&shed_proto::Input::Prompt(notes.join("\n")))
            } else {
                notes.into_iter().map(|n| n + "\r").collect()
            };
            let term = Arc::clone(term);
            std::thread::spawn(move || term.write(text.as_bytes()).ok());
        }
    }

    pub(crate) fn children(&self, name: &str) -> impl Iterator<Item = &TerminalInfo> {
        let infos = self.entries.values().map(|e| &e.info);
        infos.filter(move |t| t.parent.as_ref().is_some_and(|p| p.name == name))
    }

    pub(crate) fn busy_children(&self, name: &str) -> bool {
        let running = |t: &&TerminalInfo| t.state == State::Running;
        (self.children(name).filter(running)).any(|t| t.reported.is_none() || working(t))
    }

    pub(crate) fn orphans(&self, name: &str) -> Vec<String> {
        let ended = (self.children(name)).filter(|t| t.parent.as_ref().is_some_and(|p| !p.outlive));
        let running = ended.filter(|t| t.state == State::Running);
        running.map(|t| t.name.clone()).collect()
    }
}
