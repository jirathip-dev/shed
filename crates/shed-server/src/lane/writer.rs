use std::fs::{self, File};
use std::io::{self, BufWriter, Write as _};
use std::os::fd::{AsFd as _, OwnedFd};
use std::os::unix::fs::{DirBuilderExt as _, OpenOptionsExt as _, PermissionsExt as _};
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use shed_proto::{EmitParams, EventKind, Method, Reported, StateParams};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};

use super::Input;
use super::transcript::{Preview, stdout_style};
use crate::Client;

pub(crate) enum Out {
    Emit(EventKind, bool, bool),
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

pub(super) fn open_transcript(state: &Path, name: &str) -> io::Result<File> {
    let path = super::transcript(state, name);
    if let Some(dir) = path.parent() {
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(dir)?;
    }
    fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .mode(0o600)
        .open(&path)
}

pub(super) struct Writer {
    pub(super) transcript: BufWriter<File>,
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
                    Out::Emit(kind, replay, keep) => {
                        if keep {
                            let mut kept = kind.clone();
                            if let EventKind::Turn { agent, .. } = &mut kept {
                                agent.clear();
                            }
                            let line = serde_json::to_string(&kept).unwrap_or_default();
                            writeln!(self.transcript, "{line}").ok();
                            redraw |= self.preview.push(kept);
                        }
                        let params = EmitParams { name, kind, replay };
                        crate::notification(Method::Emit, &params)
                    }
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
            self.transcript.flush().ok();
            self.send(&calls).await;
            if let Some(tty) = self.tty.as_mut().filter(|_| redraw) {
                let (cols, rows, color) = stdout_style();
                let lines = self.preview.draw(cols, color, rows - 1).join("\r\n");
                write!(tty, "\x1b[H\x1b[2J{lines}\r\n").ok();
            }
        }
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
