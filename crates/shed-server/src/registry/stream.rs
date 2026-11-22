use super::Registry;
use crate::events::Subscriber;
use shed_core::Terminal;
use shed_proto::{ErrorCode, EventsParams, RpcError, TerminalInfo};
use std::sync::Arc;
use tokio::net::unix::OwnedWriteHalf;
use tokio::sync::mpsc::Receiver;

pub(crate) enum Stream {
    Output(Vec<u8>, Receiver<Arc<[u8]>>),
    Events(Subscriber),
    Screen(Arc<Terminal>, Option<u64>, bool),
}

impl Stream {
    pub(crate) async fn run(self, mut out: OwnedWriteHalf, beats: bool) {
        match self {
            Self::Output(first, rx) => pump(&mut out, first, rx).await,
            Self::Events(sub) => sub.stream(&mut out, beats).await,
            Self::Screen(term, since, redacted) => {
                crate::screen::watch(&term, since, redacted, &mut out).await;
            }
        }
    }
}

async fn pump(out: &mut OwnedWriteHalf, mut bytes: Vec<u8>, mut rx: Receiver<Arc<[u8]>>) {
    let mut batch = Vec::new();
    while crate::send(out, &bytes).await && rx.recv_many(&mut batch, 1024).await > 0 {
        bytes = batch.concat();
        batch.clear();
    }
}

impl Registry {
    pub(crate) fn mark_redacted(&mut self, name: &str) {
        if let Some(e) = self.entries.get_mut(name) {
            e.info.redacted = true;
        }
    }

    pub(crate) fn subscribe(&mut self, p: EventsParams) -> Stream {
        Stream::Events(self.events.subscribe(p))
    }

    pub(crate) fn attach(&self, name: &str) -> Result<(TerminalInfo, Stream), RpcError> {
        if self.entry(name)?.info.redacted {
            let why = format!("{name} is redacted: read it or watch its screen");
            return Err(RpcError::new(ErrorCode::ReadOnly, why));
        }
        let term = self.live(name)?;
        let output = term.subscribe();
        let redraw = format!("\x1b[H\x1b[2J{}", term.screen().replace('\n', "\r\n"));
        Ok((self.get(name)?, Stream::Output(redraw.into_bytes(), output)))
    }
}
