use super::{
    Entry, GRACE, Registry, handover::picked_up, run::size, run::spawn, save::FILE_VERSION,
    save::Saved, save::Saver, save::StateFile, save::write_state,
};
use crate::events::Events;
use crate::lane::sessions;
use crate::stats::Stats;
use shed_core::proctree::{self, ProcessTable as _};
use shed_core::{History, Report, Terminal};
use shed_proto::{EventKind, Reported, State, TerminalInfo};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::{self, Write};
use std::os::fd::OwnedFd;
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, UNIX_EPOCH};

pub(super) fn history(state: &Path, name: &str) -> History {
    History {
        log: Some(state.with_file_name("history").join(format!("{name}.log"))),
        ..History::default()
    }
}

pub(super) fn adopt(socket: &Path, state: &Path, info: &TerminalInfo) -> Option<Terminal> {
    let lent = crate::lane::pty_socket(socket, &info.name);
    let taken = UnixStream::connect(&lent).and_then(|bridge| {
        bridge.set_read_timeout(Some(GRACE))?;
        crate::handover::take_over(&bridge, 4096)
    });
    let Ok((Some(master), said)) = taken.map(|(fds, said)| (fds.into_iter().next(), said)) else {
        fs::remove_file(lent).ok();
        return None;
    };
    let term = Terminal::adopt(master, info.pid?, &history(state, &info.name), None).ok()?;
    term.set_idle_after(info.idle_after.map(Duration::from_secs));
    if let Ok(Some(r)) = serde_json::from_slice::<Option<Reported>>(&said) {
        term.set_report(Report {
            state: r.state,
            note: r.note,
            ..Report::default()
        });
    }
    Some(term)
}

fn end_left_over<'a>(state: &Path, terminals: impl Iterator<Item = &'a TerminalInfo>) {
    let dir = state.parent().unwrap_or(Path::new("."));
    let sample = crate::stats::system(dir);
    let booted = sample.at.saturating_sub(sample.uptime_s);
    let saved = fs::metadata(state).and_then(|m| m.modified());
    let saved = saved.ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok());
    if sample.uptime_s > 0 && saved.is_none_or(|t| t.as_secs() < booted) {
        return;
    }
    let procs = proctree::system().list().unwrap_or_default();
    for info in terminals.filter(|i| i.state == State::Running) {
        let Some(pid) = info
            .pid
            .filter(|&p| u32::try_from(p).ok() != Some(shed_core::runner::pid()))
        else {
            continue;
        };
        let left = proctree::tree(&procs, pid, &BTreeSet::new()).len();
        if left > 0 {
            proctree::kill_tree(&proctree::system(), pid, GRACE, || {});
            let name = &info.name;
            writeln!(
                io::stderr(),
                "shed serve: ended terminal {name}'s process group {pid} ({left} processes), left running by the last server"
            )
            .ok();
        }
    }
}

impl Registry {
    pub(crate) fn readable(path: &Path) -> io::Result<()> {
        serde_json::from_slice::<StateFile>(&fs::read(path)?)?;
        Ok(())
    }

    pub(crate) fn load(path: &Path, socket: &Path, masters: Vec<OwnedFd>) -> io::Result<Self> {
        let file: StateFile = match fs::read(path) {
            Ok(bytes) => serde_json::from_slice(&bytes)?,
            Err(e) if e.kind() == io::ErrorKind::NotFound => StateFile {
                version: FILE_VERSION,
                terminals: Vec::new(),
                handover: None,
            },
            Err(e) => return Err(e),
        };
        let handed = (file.handover.clone()).filter(|h| h.format == crate::handover::FORMAT);
        let mut adopted = picked_up(path, socket, &file, handed.as_ref(), masters);
        let infos = file.terminals.iter().map(|s| &s.info);
        end_left_over(path, infos.filter(|i| !adopted.contains_key(&i.name)));
        let shell = crate::config::var("SHELL").unwrap_or_else(|| "/bin/sh".to_owned());
        let mut entries = BTreeMap::new();
        let mut events = Events::default();
        events.resume(handed.as_ref().map_or(0, |h| h.seq));
        let now = shed_core::clock::unix_now();
        for Saved {
            mut info,
            exited_at,
            recipe,
            was,
            notes,
            speaks,
        } in file.terminals
        {
            let mut term = adopted.remove(&info.name).map(Arc::new);
            if info.state == State::Running && term.is_none() {
                if info.keep {
                    let cmd = vec![shell.clone()];
                    let history = history(path, &info.name);
                    term = spawn(
                        size(None, None),
                        &cmd,
                        &info.cwd,
                        info.idle_after,
                        &history,
                        &[("SHED".to_owned(), info.name.clone())],
                        false,
                    )
                    .ok()
                    .map(Arc::new);
                    if term.is_some() {
                        info.cmd = cmd;
                        events.emit(&info, EventKind::Started);
                    }
                }
                if term.is_none() {
                    (info.state, info.exit) = (State::Exited, None);
                }
            }
            info.pid = term.as_ref().map(|t| t.pid());
            let exited_at = (info.state == State::Exited).then(|| exited_at.unwrap_or(now));
            sessions::carry_over(path, &info, now);
            entries.insert(
                info.name.clone(),
                Entry {
                    exited_at,
                    notes,
                    recipe,
                    was,
                    speaks,
                    ..Entry::new(info, term)
                },
            );
        }
        for dir in ["history", "acp"] {
            for file in fs::read_dir(path.with_file_name(dir)).into_iter().flatten() {
                let Ok(file) = file else { continue };
                let stem = file
                    .path()
                    .file_stem()
                    .map(|s| s.to_string_lossy().into_owned());
                if stem.is_some_and(|s| !entries.contains_key(&s)) {
                    fs::remove_file(file.path()).ok();
                }
            }
        }
        let saver = Saver {
            dirty: Arc::default(),
            urgent: Arc::default(),
            changes: 0,
            wanted: 0,
            closing: false,
            written: tokio::sync::watch::Sender::new(0),
        };
        let registry = Self {
            entries,
            path: path.to_owned(),
            providers: path.with_file_name("providers"),
            socket: socket.to_owned(),
            saver,
            events,
            stats: Stats::load(&path.with_file_name("stats.json")),
            handed: None,
            control: None,
        };
        write_state(path, &registry.state_file())?;
        Ok(registry)
    }

    pub(crate) fn prune(&mut self, now: u64) {
        let after = crate::config::prune_after();
        let before = self.entries.len();
        let fresh = |t: u64| now.saturating_sub(t) < after;
        self.entries.retain(|_, e| e.exited_at.is_none_or(fresh));
        if self.entries.len() != before {
            self.save();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::tests::{dir, sleeper};

    use crate::lane::sessions;

    use shed_proto::SessionRecord;
    use shed_proto::{LsParams, ReadParams, Sample};

    use std::fs;
    use std::io::Write;
    use std::path::{Path, PathBuf};

    #[test]
    fn state_files_of_every_past_release_load() {
        let terminal = |name: &str, extra: &str| {
            format!(
                r#"{{"name":"{name}","labels":{{"issue":"7"}},"cwd":"/home/user","cmd":["make"],"keep":false,"state":"exited","exit":0,"pid":null{extra}}}"#
            )
        };
        let terminals = [
            terminal("m1", ""),
            terminal(
                "m2",
                r#","reported":{"state":"done","note":"ok"},"idle_after":30"#,
            ),
            terminal("m2c", r#","acp":"claude-acp""#),
            terminal(
                "osc",
                r#","reported":{"state":"blocked","state_source":"jan-osc9999"}"#,
            ),
            terminal(
                "next",
                r#","retired_key":true,"reported":{"state":"done","color":"red"}"#,
            ),
        ];
        let path = dir("releases").join("state.json");
        let file = format!(
            r#"{{"version":2,"future":[],"terminals":[{}]}}"#,
            terminals.join(",")
        );
        fs::write(&path, file).unwrap();
        let registry = Registry::load(&path, Path::new("shed.sock"), Vec::new()).unwrap();
        let names: Vec<String> = registry
            .ls(&LsParams::default())
            .into_iter()
            .map(|t| t.name)
            .collect();
        assert_eq!(names, ["m1", "m2", "m2c", "next", "osc"]);
        assert_eq!(registry.get("m2").unwrap().idle_after, Some(30));
        assert_eq!(
            registry.get("m2c").unwrap().acp.as_deref(),
            Some("claude-acp")
        );
        let osc = registry.get("osc").unwrap().reported.unwrap();
        assert_eq!(osc.state_source.as_deref(), Some("jan-osc9999"));
    }

    #[test]
    fn a_corrupt_state_file_is_refused_and_kept() {
        let path = dir("corrupt").join("state.json");
        fs::write(&path, "{\"version\":1,\"terminals\":[").unwrap();
        let e = Registry::load(&path, Path::new("shed.sock"), Vec::new())
            .err()
            .expect("refused");
        assert!(e.to_string().contains("EOF"), "{e}");
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "{\"version\":1,\"terminals\":["
        );
    }

    #[test]
    fn leftovers_go_on_load_and_a_removed_terminal_leaves_nothing() {
        let root = dir("leftovers");
        let path = root.join("state.json");
        let file = r#"{"version":1,"terminals":[{"name":"kept.1","labels":{},"cwd":"/home/user","cmd":["make"],"keep":false,"state":"exited","exit":0,"pid":null,"acp":"fake","session":{"id":"s1"}}]}"#;
        fs::write(&path, file).unwrap();
        let files = [
            "history/kept.1.log",
            "sessions/s1.jsonl",
            "history/gone.log",
            "history/gone.log.tmp",
            "acp/gone.jsonl",
        ];
        for f in files {
            fs::create_dir_all(root.join(f).parent().unwrap()).unwrap();
            fs::write(root.join(f), "x").unwrap();
        }
        let exists = || files.map(|f| root.join(f).exists());
        let mut registry = Registry::load(&path, Path::new("shed.sock"), Vec::new()).unwrap();
        assert_eq!(exists(), [true, true, false, false, false]);
        registry.rm("kept.1").unwrap().1();
        assert_eq!(exists(), [false; 5]);
    }

    #[test]
    fn a_running_old_bridge_s_transcript_is_carried_to_its_session_and_still_written() {
        let root = dir("carry");
        let path = root.join("state.json");
        let terminal = r#"{"name":"a","labels":{},"cwd":"/work","cmd":["shed"],"keep":false,"state":"running","exit":null,"pid":null,"acp":"fake","session":{"id":"s/1"}}"#;
        fs::write(
            &path,
            format!(r#"{{"version":1,"terminals":[{terminal}]}}"#),
        )
        .unwrap();
        fs::create_dir_all(root.join("acp")).unwrap();
        let line = |prompt: &str, reply: &str| {
            let turn = serde_json::json!({"event": "turn", "prompt": prompt});
            let reply =
                serde_json::json!({"event": "reply", "text": reply, "stop_reason": "end_turn"});
            format!("{turn}\n{reply}\n")
        };
        fs::write(root.join("acp/a.jsonl"), line("fix it\nplease", "done")).unwrap();
        let mut bridge = fs::OpenOptions::new()
            .append(true)
            .open(root.join("acp/a.jsonl"))
            .unwrap();
        let registry = Registry::load(&path, Path::new("shed.sock"), Vec::new()).unwrap();
        bridge.write_all(line("more", "again").as_bytes()).unwrap();
        assert!(!root.join("acp/a.jsonl").exists());
        assert!(root.join("sessions/s%2F1.jsonl").exists());
        let params = ReadParams {
            name: "a".into(),
            lines: None,
            history: false,
            events: false,
            before: None,
            n: None,
            since: None,
        };
        let read = registry.read(params).unwrap()().unwrap();
        assert_eq!(read.text, "> fix it\nplease\ndone\n> more\nagain");
        let record = sessions::read(&path, "s/1").unwrap();
        assert_eq!(
            (record.agent.as_str(), record.cwd),
            ("fake", PathBuf::from("/work"))
        );
    }

    #[test]
    fn the_prune_pass_deletes_a_session_past_the_keep_time_and_leaves_a_younger_one() {
        let root = dir("sessions-prune");
        let path = root.join("state.json");
        let mut registry = Registry::load(&path, Path::new("shed.sock"), Vec::new()).unwrap();
        let now = shed_core::clock::unix_now();
        for (id, days) in [("old", 31), ("young", 29)] {
            let updated = now - days * 86_400;
            let record = SessionRecord {
                id: id.into(),
                updated,
                ..SessionRecord::default()
            };
            sessions::write(&path, &record).unwrap();
            fs::write(sessions::transcript(&path, id), "{}\n").unwrap();
        }
        registry.record(Sample::default())();
        let left: Vec<String> = sessions::list(&path)
            .unwrap()
            .into_iter()
            .map(|r| r.id)
            .collect();
        assert_eq!(left, ["young"]);
        assert!(!sessions::transcript(&path, "old").exists());
        assert!(sessions::transcript(&path, "young").exists());
    }

    #[test]
    fn an_exited_terminal_past_the_limit_is_pruned_and_a_running_one_kept() {
        let root = dir("prune");
        let path = root.join("state.json");
        let now = shed_core::clock::unix_now();
        let terminal = |name: &str, at: u64| {
            format!(
                r#"{{"name":"{name}","labels":{{}},"cwd":"/","cmd":["make"],"keep":false,"state":"exited","exit":0,"pid":null,"exited_at":{at}}}"#
            )
        };
        let file = format!(
            r#"{{"version":1,"terminals":[{},{}]}}"#,
            terminal("old", now - 86_401),
            terminal("new", now - 60)
        );
        fs::write(&path, file).unwrap();
        fs::create_dir_all(root.join("history")).unwrap();
        fs::write(root.join("history/old.log"), "x").unwrap();
        let mut registry = Registry::load(&path, Path::new("shed.sock"), Vec::new()).unwrap();
        let term = sleeper(&mut registry, "running");
        let names = |r: &Registry| -> Vec<String> {
            r.ls(&LsParams::default())
                .into_iter()
                .map(|t| t.name)
                .collect()
        };
        assert_eq!(names(&registry), ["new", "old", "running"]);
        registry.prune(now);
        assert_eq!(names(&registry), ["new", "running"]);
        assert!(root.join("history/old.log").exists());
        registry.prune(now + 86_400);
        assert_eq!(names(&registry), ["running"]);
        term.kill(GRACE).unwrap();
    }
}
