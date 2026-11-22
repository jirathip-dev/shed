use std::sync::Arc;
use std::time::Duration;

use shed_core::clock::tokio_now;
use shed_core::{Terminal, Wake};
use shed_proto::{ErrorCode, RpcError, StateWait, TerminalRecord, Wizard};
use tokio::sync::{Mutex, OwnedMutexGuard, watch};
use tokio::time::Instant;

pub(crate) const DRAFT_WAIT: u64 = 60;
const ENDS_LINE: [u8; 4] = [b'\r', b'\n', 0x03, 0x15];

pub(crate) struct Draft {
    last_key: watch::Sender<Option<Instant>>,
    sends: Arc<Mutex<()>>,
}

impl Default for Draft {
    fn default() -> Self {
        Self {
            last_key: watch::Sender::new(None),
            sends: Arc::default(),
        }
    }
}

impl Draft {
    pub(crate) fn hold(&self) -> Option<OwnedMutexGuard<()>> {
        Arc::clone(&self.sends).try_lock_owned().ok()
    }

    async fn cleared(&self, wait: Duration, arrived: Instant) -> bool {
        let mut last_key = self.last_key.subscribe();
        loop {
            let Some(at) = *last_key.borrow_and_update() else {
                return true;
            };
            let until = at + wait;
            if arrived >= until {
                return true;
            }
            if tokio_now() >= until {
                return false;
            }
            tokio::select! {
                _ = last_key.changed() => {}
                () = tokio::time::sleep_until(until) => {}
            }
        }
    }
}

pub(crate) enum Input {
    Now(Arc<Terminal>, Vec<u8>),
    Typed(Arc<Terminal>, Vec<u8>, Arc<Draft>),
    Sent {
        name: String,
        term: Arc<Terminal>,
        bytes: Vec<u8>,
        draft: Arc<Draft>,
        wait: u64,
        prompt: Option<Box<Prompt>>,
    },
}

pub(crate) struct Prompt {
    pub(crate) record: TerminalRecord,
    pub(crate) echo: String,
    pub(crate) enter: bool,
}

async fn write(term: Arc<Terminal>, bytes: Vec<u8>) -> Result<(), RpcError> {
    let wrote = crate::call::blocking(move || term.write(&bytes)).await?;
    wrote.map_err(|e| RpcError::new(ErrorCode::Internal, e))
}

impl Input {
    pub(crate) async fn write(self) -> Result<(), RpcError> {
        match self {
            Self::Now(term, bytes) => write(term, bytes).await,
            Self::Typed(term, bytes, draft) => {
                let sent = bytes.last().is_some_and(|b| ENDS_LINE.contains(b));
                if !sent {
                    draft.last_key.send_replace(Some(tokio_now()));
                }
                write(term, bytes).await?;
                if sent {
                    draft.last_key.send_replace(None);
                }
                Ok(())
            }
            Self::Sent {
                name,
                term,
                bytes,
                draft,
                wait,
                prompt,
            } => {
                let arrived = tokio_now();
                let _turn = draft.sends.lock().await;
                if !draft.cleared(Duration::from_secs(wait), arrived).await {
                    let why = format!(
                        "{name}: a person's unsent line held this send for {wait} s; nothing was written"
                    );
                    return Err(RpcError::new(ErrorCode::DraftHeld, why));
                }
                match prompt {
                    Some(prompt) => deliver(&name, &term, bytes, *prompt).await,
                    None => write(term, bytes).await,
                }
            }
        }
    }
}

fn within(secs: u64) -> std::time::Instant {
    shed_core::clock::now() + Duration::from_secs(secs)
}

async fn reaches(term: &Terminal, wait: &StateWait) -> bool {
    let reached = |t: &Terminal| t.report().is_some_and(|r| wait.states.contains(&r.state));
    matches!(
        term.wait_until(reached, Some(within(wait.secs))).await,
        Ok(Wake::Matched)
    )
}

async fn deliver(
    name: &str,
    term: &Arc<Terminal>,
    bytes: Vec<u8>,
    p: Prompt,
) -> Result<(), RpcError> {
    let failed = |why: String| {
        Err(RpcError::new(
            ErrorCode::NotDelivered,
            format!("{name}: {why}"),
        ))
    };
    let (record, echo) = (&p.record, p.echo.as_str());
    if !reaches(term, &record.ready).await {
        return failed(format!(
            "not {} within {} s",
            record.ready.states.join(" or "),
            record.ready.secs
        ));
    }
    let shown = |t: &Terminal| t.screen().contains(echo);
    let mut echoed = false;
    for _ in 0..record.echo.attempts.max(1) {
        write(Arc::clone(term), bytes.clone()).await?;
        let wake = term.wait_until(shown, Some(within(record.echo.secs))).await;
        if matches!(wake, Ok(Wake::Matched)) {
            echoed = true;
            break;
        }
    }
    if !echoed {
        return failed(format!(
            "{echo:?} did not show after {} tries",
            record.echo.attempts.max(1)
        ));
    }
    if !p.enter {
        return Ok(());
    }
    write(Arc::clone(term), b"\r".to_vec()).await?;
    if !reaches(term, &record.accepted).await {
        let states = record.accepted.states.join(" or ");
        return failed(format!(
            "the prompt was not taken: not {states} within {} s",
            record.accepted.secs
        ));
    }
    Ok(())
}

pub(crate) async fn dismiss(
    term: Arc<Terminal>,
    wizard: Wizard,
    ready: StateWait,
    held: OwnedMutexGuard<()>,
) {
    let shows = |t: &Terminal| {
        let screen = t.screen();
        wizard.any.iter().any(|w| screen.contains(w.as_str()))
    };
    let settled =
        |t: &Terminal| shows(t) || t.report().is_some_and(|r| ready.states.contains(&r.state));
    let woke = term.wait_until(settled, Some(within(ready.secs))).await;
    if !matches!(woke, Ok(Wake::Matched)) || !shows(&term) {
        return;
    }
    for key in &wizard.keys {
        let Ok(bytes) = shed_core::keys::encode(key) else {
            continue;
        };
        if write(Arc::clone(&term), bytes).await.is_err() {
            return;
        }
    }
    tokio::time::sleep(Duration::from_secs(wizard.settle)).await;
    drop(held);
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use serde_json::{Value, json};
    use shed_proto::{Method, Request};

    use super::*;
    use crate::registry::Registry;

    async fn call(
        registry: &crate::Shared,
        method: Method,
        params: Value,
    ) -> Result<Value, RpcError> {
        let mut session = crate::call::Session {
            protocol: Some(shed_proto::PROTOCOL),
            ..crate::call::Session::default()
        };
        let request = Request::new(1, method, params);
        crate::dispatch::dispatch(registry, &mut session, &request).await
    }

    async fn shows(term: &Terminal, text: &str) {
        let shown = term.wait_until(|t| t.screen().contains(text), None).await;
        assert!(
            matches!(shown, Ok(shed_core::Wake::Matched)),
            "{}",
            term.screen()
        );
    }

    const FAKE_TUI: &str = r#"stty -echo -icanon; printf '%s\n' "$0"
key=$(dd bs=1 count=1 2>/dev/null); [ "$key" = "$(printf '\033')" ] || exit 1
stty echo icanon; printf 'dismissed\n\033]7501;state=done\007> '
read -r line; printf '\033]7501;state=working\007got: %s\n' "$line"; exec sleep 60"#;

    #[tokio::test]
    async fn a_terminal_agents_wizard_is_dismissed_and_a_send_is_typed_echoed_and_taken() {
        let dir = crate::temp_dir("tui");
        let built_in = include_str!("../../shed-catalog/providers/dsh.toml");
        let no_settle = built_in.replace("settle = 2", "settle = 0");
        assert_ne!(no_settle, built_in);
        std::fs::create_dir_all(dir.join("providers")).unwrap();
        std::fs::write(dir.join("providers/dsh.toml"), no_settle).unwrap();
        let dsh = crate::providers::Catalog::load(&dir.join("providers")).0;
        let record = dsh.by_agent("dsh").unwrap().terminal.clone().unwrap();
        let welcome = record.wizard.unwrap().any[1].clone();
        let registry =
            Registry::load(&dir.join("state.json"), &dir.join("shed.sock"), Vec::new()).unwrap();
        let registry = Arc::new(Mutex::new(registry));
        let cmd = ["sh", "-c", FAKE_TUI, welcome.as_str()];
        let run = json!({"name": "t", "cwd": dir, "cmd": cmd, "labels": {"agent": "dsh"}});
        call(&registry, Method::Run, run).await.unwrap();
        let term = Arc::clone(crate::lock(&registry).live("t").unwrap());
        let text = "deliver this prompt to the agent";
        let sent = call(&registry, Method::Send, json!({"name": "t", "text": text}));
        assert_eq!(sent.await.unwrap(), json!({}));
        shows(
            &term,
            &format!("{welcome}\ndismissed\n> {text}\ngot: {text}"),
        )
        .await;
        assert_eq!(term.report().map(|r| r.state).as_deref(), Some("working"));
        term.kill(Duration::ZERO).unwrap();
    }

    #[tokio::test(start_paused = true)]
    async fn a_send_to_a_ready_terminal_agent_with_no_wizard_starts_at_once() {
        let dir = crate::temp_dir("nowiz");
        let built_in = include_str!("../../shed-catalog/providers/dsh.toml");
        std::fs::create_dir_all(dir.join("providers")).unwrap();
        std::fs::write(dir.join("providers/dsh.toml"), built_in).unwrap();
        let registry =
            Registry::load(&dir.join("state.json"), &dir.join("shed.sock"), Vec::new()).unwrap();
        let registry = Arc::new(Mutex::new(registry));
        let ready = r#"printf '\033]7501;state=done\007> '
read -r line; printf '\033]7501;state=working\007got: %s\n' "$line"; exec sleep 60"#;
        let run = json!({"name": "t", "cwd": dir, "cmd": ["sh", "-c", ready], "labels": {"agent": "dsh"}});
        call(&registry, Method::Run, run).await.unwrap();
        let term = Arc::clone(crate::lock(&registry).live("t").unwrap());
        shows(&term, ">").await;
        let start = tokio_now();
        let sent = call(
            &registry,
            Method::Send,
            json!({"name": "t", "text": "go now"}),
        );
        assert_eq!(sent.await.unwrap(), json!({}));
        assert!(
            start.elapsed() < Duration::from_secs(30),
            "{:?}",
            start.elapsed()
        );
        shows(&term, "got: go now").await;
        term.kill(Duration::ZERO).unwrap();
    }

    #[tokio::test(start_paused = true)]
    async fn a_send_waits_for_a_persons_unsent_line_and_times_out_when_they_stay_silent() {
        let dir = crate::temp_dir("draft");
        let registry =
            Registry::load(&dir.join("state.json"), &dir.join("shed.sock"), Vec::new()).unwrap();
        let registry = Arc::new(Mutex::new(registry));
        let cmd = ["sh", "-c", "stty -echo; echo ready; exec cat"];
        let run = json!({"name": "t", "cwd": dir, "cmd": cmd});
        let term = {
            let mut reg = crate::lock(&registry);
            reg.run(serde_json::from_value(run).unwrap()).unwrap();
            Arc::clone(reg.live("t").unwrap())
        };
        shows(&term, "ready").await;
        let typed = |text: &str| {
            call(
                &registry,
                Method::Input,
                json!({"terminal": "t", "text": text}),
            )
        };
        let send = |text: &str| {
            let (registry, params) = (Arc::clone(&registry), json!({"name": "t", "text": text}));
            tokio::spawn(async move { call(&registry, Method::Send, params).await })
        };

        typed("half").await.unwrap();
        let held = send("agent");
        tokio::time::sleep(Duration::from_secs(59)).await;
        assert!(!held.is_finished());
        typed(" line\r").await.unwrap();
        assert_eq!(held.await.unwrap().unwrap(), json!({}));
        shows(&term, "ready\nhalf line\nagent").await;

        typed("draft").await.unwrap();
        let held = send("dropped");
        tokio::time::sleep(Duration::from_secs(61)).await;
        let e = held.await.unwrap().unwrap_err();
        assert_eq!(e.code, ErrorCode::DraftHeld.code());
        assert!(e.message.ends_with("for 60 s; nothing was written"), "{e}");
        typed("\r").await.unwrap();
        send("marker").await.unwrap().unwrap();
        shows(&term, "ready\nhalf line\nagent\ndraft\nmarker").await;
        assert!(!term.screen().contains("dropped"), "{}", term.screen());
    }
}
