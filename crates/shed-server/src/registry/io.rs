use super::{Registry, internal, load::history, run::size};
use crate::input::{DRAFT_WAIT, Input};
use crate::lane::sessions;
use shed_core::{Report, STATES, Screen, Size};
use shed_proto::{
    Cursor, ErrorCode, KeysParams, Logged, ReadParams, ReadResult, Reported, ResizeParams,
    RpcError, SendParams, StateParams, TerminalInfo,
};
use std::fs;
use std::sync::Arc;

pub(super) fn to_reported(r: Report) -> Reported {
    Reported {
        state: r.state,
        note: r.note,
        state_source: r.source,
        from: r.from,
        id: r.id,
        kind: r.kind,
        progress: r.progress,
        app: r.app,
        title: r.title,
        msg: r.msg,
    }
}

fn read_screen(s: &Screen, p: &ReadParams) -> ReadResult {
    let (visible, n, seq) = (s.text(), p.lines.unwrap_or(usize::MAX), s.scrolled());
    let screen = (visible.bytes()).fold(0x811c_9dc5, |h, b| {
        (h ^ u32::from(b)).wrapping_mul(0x0100_0193)
    });
    let mut read = ReadResult::default();
    read.cursor = Cursor { seq, screen };
    read.text = match p.since {
        Some(since) => {
            let (mut lines, lost) = s.scrolled_since(since.seq);
            read.changed = since.screen != screen;
            lines.extend(read.changed.then_some(visible));
            read.lost = (lost > 0).then_some(lost);
            lines.join("\n")
        }
        None if p.history => s.history(n),
        None => last_lines(&visible, n),
    };
    read
}

fn last_lines(text: &str, n: usize) -> String {
    let skip = text.lines().count().saturating_sub(n);
    text.lines().skip(skip).collect::<Vec<_>>().join("\n")
}

impl Registry {
    pub(crate) fn read(
        &self,
        p: ReadParams,
    ) -> Result<impl FnOnce() -> Result<ReadResult, RpcError> + Send + use<>, RpcError> {
        let entry = self.entry(&p.name)?;
        let (state, term) = (self.path.clone(), entry.term.clone());
        let acp = entry.info.acp.is_some();
        let id = (entry.info.session.as_ref()).map_or_else(String::new, |s| s.id.clone());
        Ok(move || {
            let dead = || history(&state, &p.name).restore(size(None, None), None);
            match &term {
                Some(term) if !acp => return Ok(term.with_screen(|s| read_screen(s, &p))),
                None if !acp => return Ok(read_screen(&dead().map_err(internal)?, &p)),
                _ => {}
            }
            let file = fs::read_to_string(sessions::transcript(&state, &id)).unwrap_or_default();
            let mut read = ReadResult::default();
            read.cursor.seq = u64::try_from(file.lines().count()).unwrap_or(u64::MAX);
            if !p.events {
                let all = crate::lane::transcript::render(&file);
                read.text = last_lines(&all, p.lines.unwrap_or(usize::MAX));
                return Ok(read);
            }
            let (before, after) = (p.before.unwrap_or(u64::MAX), p.since.map_or(0, |c| c.seq));
            let lines = (file.lines().zip(1..)).filter(|&(_, seq)| after < seq && seq < before);
            let parsed = lines.filter_map(|(l, seq)| Some((seq, serde_json::from_str(l).ok()?)));
            read.events = parsed.map(|(seq, kind)| Logged { seq, kind }).collect();
            (read.events).drain(..read.events.len().saturating_sub(p.n.unwrap_or(usize::MAX)));
            Ok(read)
        })
    }

    pub(crate) fn send(&self, p: SendParams) -> Result<Input, RpcError> {
        let term = Arc::clone(self.live(&p.name)?);
        let entry = self.entry(&p.name)?;
        let mut bytes = p.text.into_bytes();
        if entry.info.acp.is_some() {
            return Ok(Input::Now(term, bytes));
        }
        if !p.no_paste && term.bracketed_paste() {
            bytes = [b"\x1b[200~".as_slice(), &bytes, b"\x1b[201~"].concat();
        }
        if !p.no_enter {
            bytes.push(b'\r');
        }
        Ok(Input::Sent {
            name: p.name,
            term,
            bytes,
            draft: Arc::clone(&entry.draft),
            wait: p.draft_wait.unwrap_or(DRAFT_WAIT),
        })
    }

    pub(crate) fn typed(&self, p: SendParams) -> Result<Input, RpcError> {
        let term = Arc::clone(self.live(&p.name)?);
        let draft = Arc::clone(&self.entry(&p.name)?.draft);
        Ok(Input::Typed(term, p.text.into_bytes(), draft))
    }

    pub(crate) fn keys(&self, p: &KeysParams, person: bool) -> Result<Input, RpcError> {
        let term = self.live(&p.name)?;
        let bytes = p
            .keys
            .iter()
            .map(|k| shed_core::keys::encode(k))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| RpcError::new(ErrorCode::InvalidParams, e))?;
        let (term, bytes) = (Arc::clone(term), bytes.concat());
        Ok(if person {
            Input::Typed(term, bytes, Arc::clone(&self.entry(&p.name)?.draft))
        } else {
            Input::Now(term, bytes)
        })
    }

    pub(crate) fn resize(&self, p: &ResizeParams) -> Result<(), RpcError> {
        if p.rows == 0 || p.cols == 0 {
            return Err(RpcError::new(
                ErrorCode::InvalidParams,
                "rows and cols must be at least 1",
            ));
        }
        let size = Size {
            rows: p.rows,
            cols: p.cols,
        };
        self.live(&p.name)?.resize(size).map_err(internal)
    }

    pub(crate) fn state(&mut self, mut p: StateParams) -> Result<TerminalInfo, RpcError> {
        p.name = self.named(&p.name);
        let lane = crate::lane::LANE_STATES.contains(&p.report.state.as_str());
        let acp = self.entry(&p.name)?.info.acp.is_some() && lane;
        if !acp && !STATES.contains(&p.report.state.as_str()) {
            return Err(RpcError::new(
                ErrorCode::InvalidParams,
                "state must be working, blocked, done or error",
            ));
        }
        self.live(&p.name)?.set_report(Report {
            state: p.report.state,
            note: p.report.note,
            ..Report::default()
        });
        self.refresh()?;
        self.get(&p.name)
    }
}
