use super::{Entry, Registry, family::note, internal, io::to_reported, not_found};
use crate::events::Events;
use crate::lane::sessions;
use shed_core::Terminal;
use shed_core::runner::ExitStatus;
use shed_proto::{ErrorCode, EventKind, Reported, RpcError, State, TerminalInfo};
use std::fs;
use std::os::unix::process::ExitStatusExt as _;
use std::sync::Arc;

fn exit_code(status: ExitStatus) -> Option<i32> {
    status.code().or_else(|| status.signal().map(|s| 128 + s))
}

impl Registry {
    pub(crate) fn refresh(&mut self) -> Result<&mut Self, RpcError> {
        let (mut changed, mut notes) = (false, Vec::new());
        for entry in self.entries.values_mut() {
            let Some(term) = &entry.term else { continue };
            let running = entry.info.state == State::Running;
            let exit = if running {
                term.try_wait().map_err(internal)?
            } else {
                None
            };
            let reported = term.report().map(to_reported);
            let records: Vec<Reported> = term.records().into_iter().map(to_reported).collect();
            if let Some(r) = reported
                .as_ref()
                .filter(|r| entry.info.reported.as_ref() != Some(*r))
                && ["blocked", "waiting-approval", "rate-limited", "error"]
                    .contains(&r.state.as_str())
            {
                let what = [Some(&r.state), r.note.as_ref()].into_iter().flatten();
                notes.extend(note(
                    &entry.info,
                    &what.cloned().collect::<Vec<_>>().join(": "),
                ));
            }
            let news = records.iter().filter(|r| !entry.info.records.contains(r));
            let news = reported
                .iter()
                .filter(|r| entry.info.reported.as_ref() != Some(*r))
                .chain(news);
            for r in news {
                self.events
                    .emit(&entry.info, EventKind::State(Box::new(r.clone())));
            }
            if (&entry.info.reported, &entry.info.records) != (&reported, &records) {
                (entry.info.reported, entry.info.records) = (reported, records);
                changed = true;
            }
            if let Some(status) = exit {
                notes.extend(Self::exited(&mut self.events, entry, status));
                changed = true;
            }
        }
        self.deliver(notes);
        if changed {
            self.save();
        }
        Ok(self)
    }

    fn exited(
        events: &mut Events,
        entry: &mut Entry,
        status: ExitStatus,
    ) -> Option<(String, String)> {
        let code = exit_code(status);
        (entry.info.state, entry.info.exit) = (State::Exited, code);
        entry.info.pid = None;
        entry.exited_at = Some(shed_core::clock::unix_now());
        entry.resources = None;
        if let Some(term) = &entry.term {
            entry.info.reported = term.report().map(to_reported);
            entry.info.records = term.records().into_iter().map(to_reported).collect();
        }
        let (kind, what) = match code {
            _ if entry.killing => (EventKind::Killed, "killed".to_owned()),
            Some(n) => (EventKind::Exited { code }, format!("exited {n}")),
            None => (EventKind::Exited { code }, "exited".to_owned()),
        };
        events.emit(&entry.info, kind);
        note(&entry.info, &what)
    }

    pub(crate) fn release_lanes(&self) -> Vec<Arc<Terminal>> {
        let (lanes, rest): (Vec<_>, Vec<_>) = (self.entries.values())
            .filter(|e| e.info.state == State::Running)
            .filter_map(|e| Some((e.info.acp.is_some(), e.term.clone()?)))
            .partition(|(lane, _)| *lane);
        for (_, term) in &lanes {
            term.release();
        }
        rest.into_iter().map(|(_, term)| term).collect()
    }

    pub(crate) fn running(&self) -> Vec<(String, Arc<Terminal>)> {
        self.entries
            .values()
            .filter(|e| e.info.state == State::Running)
            .filter_map(|e| Some((e.info.name.clone(), e.term.clone()?)))
            .collect()
    }

    pub(crate) fn to_wait(
        &self,
        name: &str,
    ) -> Result<(Option<Arc<Terminal>>, TerminalInfo), RpcError> {
        let entry = self.entry(name)?;
        Ok((entry.term.clone(), entry.info.clone()))
    }

    pub(crate) fn mark_kill(&mut self, name: &str) -> Result<Option<Arc<Terminal>>, RpcError> {
        self.refresh()?;
        let entry = self.entries.get_mut(name).ok_or_else(|| not_found(name))?;
        entry.killing = entry.info.state == State::Running;
        Ok(entry.term.clone().filter(|_| entry.killing))
    }

    pub(crate) fn killed(
        &mut self,
        name: &str,
        term: &Arc<Terminal>,
        status: ExitStatus,
    ) -> Result<TerminalInfo, RpcError> {
        let entry = self.entries.get_mut(name).ok_or_else(|| not_found(name))?;
        if entry.info.state == State::Running
            && entry.term.as_ref().is_some_and(|t| Arc::ptr_eq(t, term))
        {
            let note = Self::exited(&mut self.events, entry, status);
            self.deliver(note.into_iter().collect());
        }
        let info = self.entry(name)?.view();
        Ok(self.saved(info))
    }

    pub(crate) fn rm(
        &mut self,
        name: &str,
    ) -> Result<(TerminalInfo, impl FnOnce() + Send + use<>), RpcError> {
        if self.entry(name)?.info.state == State::Running {
            return Err(RpcError::new(ErrorCode::StillRunning, name));
        }
        let entry = self.entries.remove(name).ok_or_else(|| not_found(name))?;
        let files = self.files(name);
        let session = (entry.info.session.as_ref()).filter(|_| entry.info.acp.is_some());
        let (state, session) = (self.path.clone(), session.map(|s| s.id.clone()));
        let named = name.to_owned();
        Ok((self.saved(entry.info), move || {
            crate::sandbox::remove(&state, &named);
            for file in files.into_iter().flatten() {
                fs::remove_file(file).ok();
            }
            if let Some(id) = session {
                sessions::remove(&state, &id);
            }
        }))
    }
}
