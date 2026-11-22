use super::{Entry, Registry, handover::Handed};
use serde::{Deserialize, Serialize};
use shed_proto::TerminalInfo;
use std::cell::Cell;
use std::fs;
use std::io::{self, Write};
use std::os::unix::fs::{DirBuilderExt as _, OpenOptionsExt as _};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::Notify;

pub(super) const FILE_VERSION: u32 = 1;

pub(super) const SAVE_EVERY: Duration = Duration::from_millis(500);

#[derive(Serialize, Deserialize)]
pub(super) struct StateFile {
    pub(super) version: u32,
    pub(super) terminals: Vec<Saved>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) handover: Option<Handed>,
}

#[derive(Serialize, Deserialize)]
pub(super) struct Saved {
    #[serde(flatten)]
    pub(super) info: TerminalInfo,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) exited_at: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) recipe: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) was: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(super) notes: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) speaks: Option<u32>,
}

pub(crate) struct Saver {
    pub(super) dirty: Arc<Notify>,
    pub(super) urgent: Arc<Notify>,
    pub(super) changes: u64,
    pub(super) wanted: u64,
    pub(super) closing: bool,
    pub(super) written: tokio::sync::watch::Sender<u64>,
}

impl Saver {
    fn changed(&mut self, urgent: bool) {
        self.changes += 1;
        self.dirty.notify_one();
        if urgent {
            self.wanted = self.changes;
            CHANGED.try_with(|c| c.set(self.changes)).ok();
            self.urgent.notify_one();
        }
    }

    pub(crate) fn flushed(&self) -> impl Future<Output = ()> + Send + use<> {
        self.flushed_to(self.wanted)
    }

    pub(crate) fn flushed_to(&self, wanted: u64) -> impl Future<Output = ()> + Send + use<> {
        let mut written = self.written.subscribe();
        async move {
            written.wait_for(|w| *w >= wanted).await.ok();
        }
    }
}

tokio::task_local! {
    static CHANGED: Cell<u64>;
}

pub(crate) async fn answering<T>(request: impl Future<Output = T>) -> (T, u64) {
    CHANGED
        .scope(Cell::new(0), async {
            (request.await, CHANGED.with(Cell::get))
        })
        .await
}

pub(crate) async fn keep_saved(registry: Arc<Mutex<Registry>>) {
    let (dirty, urgent) = {
        let saver = &crate::lock(&registry).saver;
        (Arc::clone(&saver.dirty), Arc::clone(&saver.urgent))
    };
    loop {
        dirty.notified().await;
        let (path, file, generation, closing) = {
            let reg = crate::lock(&registry);
            let saver = &reg.saver;
            (
                reg.path.clone(),
                reg.state_file(),
                saver.changes,
                saver.closing,
            )
        };
        let write = move || write_state(&path, &file).ok();
        tokio::task::spawn_blocking(write).await.ok();
        crate::lock(&registry)
            .saver
            .written
            .send_replace(generation);
        if closing {
            return;
        }
        tokio::select! {
            () = tokio::time::sleep(SAVE_EVERY) => {}
            () = urgent.notified() => {}
        }
    }
}

pub(super) fn write_state(path: &Path, file: &StateFile) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(dir)?;
    }
    let tmp = path.with_extension("json.tmp");
    let mut f = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&tmp)?;
    serde_json::to_writer(&mut f, file)?;
    f.write_all(b"\n")?;
    f.sync_all()?;
    fs::rename(&tmp, path)
}

impl Entry {
    fn saved(&self) -> Saved {
        let mut info = self.info.clone();
        if let Some(session) = &mut info.session {
            (session.commands, session.models) = (Vec::new(), Vec::new());
        }
        Saved {
            info,
            exited_at: self.exited_at,
            recipe: self.recipe.clone(),
            was: self.was.clone(),
            notes: self.notes.clone(),
            speaks: self.speaks,
        }
    }
}

impl Registry {
    pub(super) fn state_file(&self) -> StateFile {
        StateFile {
            version: FILE_VERSION,
            terminals: self.entries.values().map(Entry::saved).collect(),
            handover: self.handed.clone(),
        }
    }

    pub(super) fn save(&mut self) {
        self.saver.changed(false);
    }

    pub(super) fn saved<T>(&mut self, value: T) -> T {
        self.saver.changed(true);
        value
    }

    pub(crate) fn close(&mut self) -> impl Future<Output = ()> + Send + use<> {
        self.saver.closing = true;
        self.saver.changed(true);
        self.saver.flushed()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::save::SAVE_EVERY;
    use crate::registry::tests::{dir, saved, sleeper};
    use crate::registry::*;

    use shed_proto::{Reported, StateParams};

    use std::fs;

    use std::path::Path;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    #[tokio::test(start_paused = true)]
    async fn a_thousand_changes_in_a_second_write_at_most_three_times_and_end_on_disk() {
        let path = dir("burst").join("state.json");
        let registry = Arc::new(Mutex::new(
            Registry::load(&path, Path::new("shed.sock"), Vec::new()).unwrap(),
        ));
        tokio::spawn(keep_saved(Arc::clone(&registry)));
        let term = sleeper(&mut crate::lock(&registry), "busy");
        let flushed = crate::lock(&registry).saver.flushed();
        flushed.await;
        let stamp = || {
            let meta = fs::metadata(&path).unwrap();
            (meta.modified().unwrap(), fs::read(&path).unwrap())
        };
        let (mut last, mut writes) = (stamp(), 0);
        for n in 0..1000 {
            let report = Reported {
                state: (if n % 2 == 0 { "working" } else { "blocked" }).to_owned(),
                note: Some(format!("change {n}")),
                ..Reported::default()
            };
            let name = "busy".to_owned();
            crate::lock(&registry)
                .state(StateParams { name, report })
                .unwrap();
            tokio::time::sleep(Duration::from_millis(1)).await;
            if stamp() != last {
                (last, writes) = (stamp(), writes + 1);
            }
        }
        tokio::time::sleep(SAVE_EVERY).await;
        if stamp() != last {
            writes += 1;
        }
        assert!((1..=3).contains(&writes), "{writes} writes");
        let file = saved(&path);
        let busy = &file["terminals"][0];
        assert_eq!(busy["reported"]["note"], "change 999");
        assert_eq!(busy["reported"]["state"], "blocked");
        term.kill(GRACE).unwrap();
    }

    #[test]
    fn seventy_acp_terminals_save_under_300_kb_without_commands_or_models() {
        let path = dir("acp-size").join("state.json");
        let commands: Vec<_> = (0..100)
            .map(|n| [format!("command-{n}"), "d".repeat(170), "hint".to_owned()])
            .collect();
        let models = vec![["model-id".to_owned(), "Model".to_owned()]; 20];
        let terminals: Vec<_> = (0..70)
            .map(|n| {
                serde_json::json!({
                    "name": format!("acp{n}"), "labels": {"issue": "7"}, "cwd": "/home/user",
                    "cmd": ["shed", "acp-bridge"], "keep": false, "state": "exited", "exit": 0,
                    "pid": null, "acp": "agent",
                    "session": {"id": format!("s{n}"), "model": "model-id", "models": models, "commands": commands},
                })
            })
            .collect();
        let old = serde_json::json!({"version": 1, "terminals": terminals});
        fs::write(&path, serde_json::to_vec_pretty(&old).unwrap()).unwrap();
        assert!(fs::metadata(&path).unwrap().len() > 1_000_000);
        let registry = Registry::load(&path, Path::new("shed.sock"), Vec::new()).unwrap();
        assert!(fs::metadata(&path).unwrap().len() < 300_000);
        let file = saved(&path);
        let acp69 = file["terminals"]
            .as_array()
            .unwrap()
            .iter()
            .find(|t| t["name"] == "acp69");
        let session = &acp69.unwrap()["session"];
        assert_eq!(
            *session,
            serde_json::json!({"id": "s69", "model": "model-id"})
        );
        let live = registry.get("acp69").unwrap().session.unwrap();
        assert_eq!(live.commands.len(), 100);
    }
}
