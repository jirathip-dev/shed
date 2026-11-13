use std::sync::Arc;
use std::time::Duration;

use shed_core::Terminal;
use shed_core::clock::tokio_now;
use shed_proto::{ErrorCode, RpcError};
use tokio::sync::{Mutex, watch};
use tokio::time::Instant;

pub(crate) const DRAFT_WAIT: u64 = 60;
const ENDS_LINE: [u8; 4] = [b'\r', b'\n', 0x03, 0x15];

pub(crate) struct Draft {
    last_key: watch::Sender<Option<Instant>>,
    sends: Mutex<()>,
}

impl Default for Draft {
    fn default() -> Self {
        Self {
            last_key: watch::Sender::new(None),
            sends: Mutex::new(()),
        }
    }
}

impl Draft {
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
    },
}

async fn write(term: Arc<Terminal>, bytes: Vec<u8>) -> Result<(), RpcError> {
    let wrote = crate::blocking(move || term.write(&bytes)).await?;
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
            } => {
                let arrived = tokio_now();
                let _turn = draft.sends.lock().await;
                if !draft.cleared(Duration::from_secs(wait), arrived).await {
                    let why = format!(
                        "{name}: a person's unsent line held this send for {wait} s; nothing was written"
                    );
                    return Err(RpcError::new(ErrorCode::DraftHeld, why));
                }
                write(term, bytes).await
            }
        }
    }
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
        crate::input(registry, method, &Request::new(1, method, params)).await
    }

    async fn shows(term: &Terminal, text: &str) {
        let shown = term.wait_until(|t| t.screen().contains(text), None).await;
        assert!(
            matches!(shown, Ok(shed_core::Wake::Matched)),
            "{}",
            term.screen()
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_send_waits_for_a_persons_unsent_line_and_times_out_when_they_stay_silent() {
        let dir = crate::temp_dir("draft");
        let registry = Registry::load(&dir.join("state.json"), &dir.join("shed.sock")).unwrap();
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
