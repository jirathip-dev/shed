use std::fs::{self, File};
use std::io::{self, BufWriter, Write as _};
use std::os::fd::{AsFd as _, OwnedFd};
use std::os::unix::fs::PermissionsExt as _;
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use shed_proto::{EmitParams, EventKind, Method, Reported, SessionRecord, StateParams};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};

use super::transcript::{Preview, stdout_style};
use super::{Input, sessions};
use crate::Client;

pub(crate) enum Out {
    Emit(EventKind),
    Session(String, bool),
    State(Reported),
    Say(String),
    Name(String),
}

pub(super) fn lend(
    master: OwnedFd,
    at: &Path,
    reported: Arc<Mutex<Option<Reported>>>,
) -> io::Result<()> {
    fs::remove_file(at).ok();
    let listener = UnixListener::bind(at)?;
    fs::set_permissions(at, fs::Permissions::from_mode(0o600))?;
    std::thread::spawn(move || {
        for server in listener.incoming().flatten() {
            let said = serde_json::to_vec(&*crate::lock(&reported)).unwrap_or_default();
            crate::handover::hand_over(&server, &[master.as_fd()], &said).ok();
        }
    });
    Ok(())
}

pub(super) struct Writer {
    pub(super) transcript: Option<BufWriter<File>>,
    pub(super) state: PathBuf,
    pub(super) record: SessionRecord,
    pub(super) tty: Option<File>,
    pub(super) reported: Arc<Mutex<Option<Reported>>>,
    pub(super) server: Option<tokio::net::UnixStream>,
    pub(super) socket: PathBuf,
    pub(super) name: String,
    pub(super) preview: Preview,
}

impl Writer {
    pub(super) async fn run(mut self, mut outs: UnboundedReceiver<Out>) {
        let mut batch = Vec::new();
        let mut beat = tokio::time::interval(crate::HEARTBEAT);
        let heartbeat = crate::notification(Method::Heartbeat, &serde_json::json!({}));
        loop {
            tokio::select! {
                n = outs.recv_many(&mut batch, 4096) => if n == 0 { return },
                _ = beat.tick() => {
                    self.send(heartbeat.as_deref().unwrap_or_default()).await;
                    continue;
                }
            }
            let (mut redraw, mut calls) = (false, Vec::new());
            for out in batch.drain(..) {
                let name = self.name.clone();
                let call = match out {
                    Out::Emit(kind) => {
                        redraw |= self.keep(&kind);
                        let replay = false;
                        crate::notification(Method::Emit, &EmitParams { name, kind, replay })
                    }
                    Out::Session(id, resumed) if id != self.record.id => {
                        redraw = true;
                        Ok(self.open(&id, resumed))
                    }
                    Out::Session(..) => Ok(Vec::new()),
                    Out::State(report) => {
                        *crate::lock(&self.reported) = Some(report.clone());
                        let params = StateParams { name, report };
                        crate::notification(Method::State, &params)
                    }
                    Out::Name(name) => {
                        self.name = name;
                        Ok(Vec::new())
                    }
                    Out::Say(line) => {
                        self.preview.blocks.push(Err(line));
                        redraw = true;
                        Ok(Vec::new())
                    }
                };
                calls.extend(call.unwrap_or_default());
            }
            if let Some(transcript) = &mut self.transcript {
                transcript.flush().ok();
            }
            self.send(&calls).await;
            if let Some(tty) = self.tty.as_mut().filter(|_| redraw) {
                let (cols, rows, color) = stdout_style();
                let lines = self.preview.draw(cols, color, rows - 1).join("\r\n");
                write!(tty, "\x1b[H\x1b[2J{lines}\r\n").ok();
            }
        }
    }

    fn keep(&mut self, kind: &EventKind) -> bool {
        let mut kept = kind.clone();
        match &mut kept {
            EventKind::Turn { agent, prompt } => {
                agent.clear();
                if self.record.title.is_empty() {
                    self.record.title = sessions::title(prompt);
                }
            }
            EventKind::Reply { .. } => {
                (self.record.turns, self.record.updated) =
                    (self.record.turns + 1, shed_core::clock::unix_now());
                sessions::write(&self.state, &self.record).ok();
            }
            _ => {}
        }
        if let Some(transcript) = &mut self.transcript {
            let line = serde_json::to_string(&kept).unwrap_or_default();
            writeln!(transcript, "{line}").ok();
        }
        self.preview.push(kept)
    }

    fn open(&mut self, id: &str, resumed: bool) -> Vec<u8> {
        let path = sessions::transcript(&self.state, id);
        let had = (fs::read_to_string(&path).ok()).filter(|_| resumed);
        let known = sessions::read(&self.state, id).filter(|_| resumed);
        self.record = known.unwrap_or_else(|| SessionRecord {
            id: id.to_owned(),
            agent: std::mem::take(&mut self.record.agent),
            cwd: std::mem::take(&mut self.record.cwd),
            updated: shed_core::clock::unix_now(),
            ..SessionRecord::default()
        });
        self.transcript = (sessions::create(&path, resumed).ok()).map(BufWriter::new);
        sessions::write(&self.state, &self.record).ok();
        let (mut calls, had) = (Vec::new(), had.unwrap_or_default());
        for kind in had.lines().filter_map(|l| serde_json::from_str(l).ok()) {
            self.preview.push(EventKind::clone(&kind));
            let (name, replay) = (self.name.clone(), true);
            let params = EmitParams { name, kind, replay };
            calls.extend(crate::notification(Method::Emit, &params).unwrap_or_default());
        }
        calls
    }

    async fn send(&mut self, calls: &[u8]) {
        for _ in 0..2 {
            if self.server.is_none() {
                let socket = Client::connect(&self.socket).ok().map(|c| c.split().1);
                let socket = socket.filter(|s| s.set_nonblocking(true).is_ok());
                self.server = socket.and_then(|s| tokio::net::UnixStream::from_std(s).ok());
            }
            let Some(server) = &mut self.server else {
                return;
            };
            let wrote = tokio::time::timeout(crate::WRITE_TIMEOUT, server.write_all(calls)).await;
            if matches!(wrote, Ok(Ok(()))) {
                return;
            }
            self.server = None;
        }
    }
}

pub(super) async fn read_input(inputs: UnboundedSender<Input>) {
    let mut stdin = tokio::io::stdin();
    let mut line = Vec::new();
    let mut buf = [0u8; 4096];
    while let Ok(n @ 1..) = stdin.read(&mut buf).await {
        for &b in buf.iter().take(n) {
            let input = match b {
                0x1b | 0x03 => {
                    line.clear();
                    Some(Input::Cancel)
                }
                b'\r' | b'\n' => {
                    let text = String::from_utf8_lossy(&line).into_owned();
                    line.clear();
                    match text.strip_prefix('\x1e') {
                        Some(json) => serde_json::from_str(json).ok(),
                        None if text.trim().is_empty() => None,
                        None => Some(Input::Prompt(text)),
                    }
                }
                0x7f => {
                    line.pop();
                    None
                }
                b => {
                    line.push(b);
                    None
                }
            };
            if let Some(input) = input
                && inputs.send(input).is_err()
            {
                return;
            }
        }
    }
}
