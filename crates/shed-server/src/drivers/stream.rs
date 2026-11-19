use std::collections::VecDeque;
use std::fs;
use std::io::{Read as _, Write as _};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use serde_json::{Map, Value};
use shed_proto::{DriverKind, Provider, Stream};
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader, Lines};
use tokio::process::{Child, ChildStdout};
use tokio::sync::mpsc::{UnboundedSender, unbounded_channel};

use crate::Error;
use crate::lane::{Driver, Heard, Opened, Start, Stop, sessions};

pub(crate) trait Wire: Send {
    fn prompt(&self, text: &str) -> String;
    fn heard(&mut self, line: &Value, record: &Stream) -> Vec<Heard>;
    fn mcp(&self, servers: &[Value]) -> String;
}

pub(crate) fn choose(
    record: &Provider,
    policy: &str,
    cmd: &[String],
    installed: Option<&[String]>,
    meta: Option<&Map<String, Value>>,
) -> Result<Vec<String>, String> {
    let stream = (record.stream.as_ref())
        .filter(|_| record.driver == DriverKind::Stream)
        .ok_or("the agent's record does not turn the stream driver on")?;
    let flags = (stream.policy.get(policy))
        .ok_or_else(|| format!("policy {policy} has no flags on the stream driver"))?;
    let cmd = (Some(cmd).filter(|c| !c.is_empty()).or(installed)).unwrap_or_default();
    let wrapper = wrapper(stream, cmd).ok_or("the command is not the agent's ACP adapter")?;
    let mut options = Vec::new();
    for (key, value) in meta.into_iter().flatten() {
        option(stream, key, value, &mut options)?;
    }
    Ok([wrapper, &stream.command, flags, &options].concat())
}

fn wrapper<'a>(stream: &Stream, cmd: &'a [String]) -> Option<&'a [String]> {
    let Some((last, before)) = cmd.split_last() else {
        return Some(&[]);
    };
    let adapter = stream.adapter.as_deref()?;
    if !last.contains(adapter) || last.contains(char::is_whitespace) {
        return None;
    }
    let launchers = (before.iter().rev()).take_while(|w| stream.launchers.contains(w));
    before.get(..before.len() - launchers.count())
}

fn option(stream: &Stream, at: &str, value: &Value, out: &mut Vec<String>) -> Result<(), String> {
    let inner = format!("{at}.");
    let flag = stream.meta.get(at).cloned();
    match (value, flag) {
        (Value::Object(map), _) if stream.meta.keys().any(|k| k.starts_with(&inner)) => {
            for (key, value) in map {
                option(stream, &format!("{inner}{key}"), value, out)?;
            }
        }
        (_, None) => {
            return Err(format!(
                "--meta option {at} has no flag on the stream driver"
            ));
        }
        (Value::Null | Value::Bool(false), _) => {}
        (Value::Bool(true), Some(flag)) => out.push(flag),
        (Value::String(text), Some(flag)) => out.extend([flag, text.clone()]),
        (Value::Array(list), Some(flag)) => {
            let text = |v: &Value| v.as_str().map_or_else(|| v.to_string(), str::to_owned);
            out.extend([flag, list.iter().map(text).collect::<Vec<_>>().join(",")]);
        }
        (value, Some(flag)) => out.extend([flag, value.to_string()]),
    }
    Ok(())
}

fn mint() -> Result<String, String> {
    let mut bytes = [0u8; 16];
    let random = fs::File::open("/dev/urandom").and_then(|mut f| f.read_exact(&mut bytes));
    random.map_err(|e| format!("/dev/urandom: {e}"))?;
    let uuid = u128::from_be_bytes(bytes) & !(0xf << 76 | 0x3 << 62) | (0x4 << 76 | 0x2 << 62);
    let hex = format!("{uuid:032x}");
    let parts = [(0, 8), (8, 12), (12, 16), (16, 20), (20, 32)];
    let parts = parts.map(|(from, to)| hex.get(from..to).unwrap_or_default());
    Ok(parts.join("-"))
}

#[derive(Default)]
pub(crate) struct Launch {
    record: Stream,
    state: PathBuf,
    mcp: Vec<Value>,
    shown: Vec<Value>,
    model: Option<String>,
    ended: bool,
    cmd: Vec<String>,
    cwd: PathBuf,
    session: String,
    resumed: bool,
}

impl Launch {
    pub(crate) fn from_env(
        cmd: &[String],
        record: Option<Stream>,
        state: &Path,
    ) -> Result<Self, Error> {
        let mcp = crate::config::var("SHED_ACP_MCP").unwrap_or_default();
        let mcp: Vec<Value> = serde_json::from_str(&mcp).unwrap_or_default();
        let shown = super::acp::mcp_servers(&mcp).map_err(Error::Acp)?;
        Ok(Self {
            record: record.unwrap_or_default(),
            state: state.to_owned(),
            cmd: cmd.to_vec(),
            shown: super::acp::redacted(&shown),
            mcp,
            ..Self::default()
        })
    }
}

pub(crate) struct Agent {
    launch: Launch,
    wire: Box<dyn Wire>,
    child: Option<Child>,
    input: Option<UnboundedSender<String>>,
    lines: Option<Lines<BufReader<ChildStdout>>>,
    stirred: Arc<AtomicBool>,
    said: bool,
    sent: Vec<String>,
    taken: usize,
    held: VecDeque<Heard>,
}

impl Agent {
    fn file(&self) -> PathBuf {
        sessions::mcp(&self.launch.state, &self.launch.session)
    }

    fn spawn(&mut self) -> Result<(), String> {
        let (record, launch) = (&self.launch.record, &self.launch);
        let with = |args: &[String], key: &str, value: &str| -> Vec<String> {
            args.iter().map(|a| a.replace(key, value)).collect()
        };
        let start = launch.resumed.then_some(&record.resume);
        let start = with(start.unwrap_or(&record.new), "{session}", &launch.session);
        let mut args = [launch.cmd.clone(), start].concat();
        if let Some(model) = &launch.model {
            args.extend(with(&record.model, "{model}", model));
        }
        if !launch.mcp.is_empty() {
            let file = self.file();
            let text = self.wire.mcp(&launch.mcp);
            let wrote =
                sessions::create(&file, false).and_then(|mut f| f.write_all(text.as_bytes()));
            wrote.map_err(|e| e.to_string())?;
            args.extend(with(&record.mcp, "{file}", &file.to_string_lossy()));
        }
        let (program, args) = args.split_first().ok_or("empty agent command")?;
        let unset = ["SHED_ACP_MCP", "SHED_ACP_META"];
        let mut child = shed_core::runner::piped(program, args, &launch.cwd, &unset)
            .map_err(|e| format!("{program}: {e}"))?;
        let (Some(mut stdin), Some(stdout), Some(stderr)) =
            (child.stdin.take(), child.stdout.take(), child.stderr.take())
        else {
            return Err("the agent's stdio is not piped".to_owned());
        };
        crate::lane::echo(stderr, Arc::clone(&self.stirred));
        (self.lines, self.said) = (Some(BufReader::new(stdout).lines()), false);
        let (input, mut prompts) = unbounded_channel::<String>();
        tokio::spawn(async move {
            while let Some(line) = prompts.recv().await
                && stdin
                    .write_all(format!("{line}\n").as_bytes())
                    .await
                    .is_ok()
            {}
        });
        for line in &self.sent {
            input.send(line.clone()).ok();
        }
        (self.child, self.input) = (Some(child), Some(input));
        Ok(())
    }

    fn hear(&mut self, line: &str) {
        let heard = match serde_json::from_str(line) {
            Ok(line) => self.wire.heard(&line, &self.launch.record),
            Err(e) => vec![Heard::Skipped(e.to_string())],
        };
        for heard in heard {
            match heard {
                Heard::Prompted => self.taken += 1,
                Heard::TurnEnd { .. } if self.taken == 0 => {}
                end @ Heard::TurnEnd { .. } => {
                    (self.taken, self.launch.ended) = (self.taken - 1, true);
                    self.held.push_back(end);
                }
                heard => self.held.push_back(heard),
            }
        }
    }
}

impl Driver for Agent {
    const NAME: &'static str = "stream";

    async fn open(&mut self, cwd: &Path, start: &Start) -> Result<Opened, String> {
        let session = Some(start.session.clone()).filter(|s| !s.is_empty());
        self.launch.session = session.map_or_else(mint, Ok)?;
        (self.launch.cwd, self.launch.resumed) = (cwd.to_owned(), start.resumed);
        self.spawn()?;
        let pid = (self.child.as_ref()).and_then(|c| c.id().and_then(|p| i32::try_from(p).ok()));
        let (model, models) = (
            self.launch.model.clone(),
            Some(self.launch.record.models.clone()),
        );
        Ok(Opened {
            id: self.launch.session.clone(),
            resumable: true,
            mcp: self.launch.shown.clone(),
            session: Heard::Session {
                model,
                models,
                commands: None,
            },
            pid,
            stirred: Arc::clone(&self.stirred),
        })
    }

    async fn heard(&mut self) -> Option<Heard> {
        loop {
            if let Some(heard) = self.held.pop_front() {
                return Some(heard);
            }
            let line = match &mut self.lines {
                Some(lines) => lines.next_line().await.ok().flatten(),
                None => None,
            };
            self.stirred.store(true, Ordering::Relaxed);
            if !self.said {
                fs::remove_file(self.file()).ok();
            }
            match line {
                Some(line) => {
                    (self.said, self.sent) = (true, Vec::new());
                    self.hear(&line);
                }
                None if self.launch.resumed && !self.launch.ended => {
                    self.launch.resumed = false;
                    self.spawn().ok()?;
                }
                None => return None,
            }
        }
    }

    fn gone(&self) -> impl Future<Output = ()> + 'static {
        std::future::pending()
    }

    fn prompt(&mut self, text: &str) -> Result<(), String> {
        let line = self.wire.prompt(text);
        if !self.said {
            self.sent.push(line.clone());
        }
        let input = self.input.as_ref().ok_or("the agent is not started")?;
        input
            .send(line)
            .map_err(|_| "the agent's stdin is closed".to_owned())
    }

    fn cancel(&mut self) -> Result<Stop, String> {
        Ok(Stop::Yes)
    }

    fn model(&mut self, model: &str) -> Result<Stop, String> {
        self.launch.model = Some(model.to_owned());
        Ok(Stop::Yes)
    }

    fn approve(&mut self, _: &str, _: Option<&str>) -> Result<(), String> {
        Ok(())
    }

    fn forget(&mut self) {
        self.taken = 0;
    }
}

pub(crate) async fn run<T>(
    launch: &mut Launch,
    main: impl AsyncFnOnce(&mut Agent) -> Result<T, String>,
) -> Result<(Child, T), Error> {
    let wire = crate::providers::wire(&launch.record.wire);
    let wire = wire.ok_or_else(|| Error::Acp(format!("no wire {:?}", launch.record.wire)))?;
    let mut agent = Agent {
        launch: std::mem::take(launch),
        wire,
        child: None,
        input: None,
        lines: None,
        stirred: Arc::default(),
        said: false,
        sent: Vec::new(),
        taken: 0,
        held: VecDeque::new(),
    };
    let done = main(&mut agent).await;
    *launch = std::mem::take(&mut agent.launch);
    let done = done.map_err(Error::Acp)?;
    let child = agent
        .child
        .take()
        .ok_or_else(|| Error::Acp("the agent never started".to_owned()))?;
    Ok((child, done))
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};
    use shed_proto::{DriverKind, Provider, Stream};

    use super::choose;

    fn words(text: &[&str]) -> Vec<String> {
        text.iter().map(|&w| w.to_owned()).collect()
    }

    fn record() -> Provider {
        let meta = [
            ("system", "--system"),
            ("system.append", "--append"),
            ("opts.sources", "--sources"),
            ("opts.strict", "--strict"),
            ("opts.loose", "--loose"),
            ("opts.settings", "--settings"),
            ("opts.turns", "--turns"),
        ];
        let stream = Stream {
            command: words(&["agent", "-p"]),
            adapter: Some("agent-acp".to_owned()),
            launchers: words(&["npx", "-y", "node"]),
            policy: [("allow".to_owned(), words(&["--yes"]))].into(),
            meta: meta.map(|(k, v)| (k.to_owned(), v.to_owned())).into(),
            ..Stream::default()
        };
        Provider {
            driver: DriverKind::Stream,
            stream: Some(stream),
            ..Provider::default()
        }
    }

    fn chose(
        record: &Provider,
        policy: &str,
        cmd: &[&str],
        meta: &Value,
    ) -> Result<Vec<String>, String> {
        let meta = meta.as_object().filter(|m| !m.is_empty());
        choose(record, policy, &words(cmd), None, meta)
    }

    #[test]
    fn a_lane_stays_on_acp_for_each_reason_it_cannot_be_served_whole() {
        let off = Err("the agent's record does not turn the stream driver on".to_owned());
        let (records, _) = crate::providers::load(&crate::providers::tests::fake_dir());
        let built_in: Vec<_> = records.iter().filter(|(_, s)| s == "built-in").collect();
        assert!(!built_in.is_empty());
        for (record, _) in built_in {
            assert_eq!(
                chose(record, "allow", &[], &json!({})),
                off,
                "{}",
                record.id
            );
        }
        let no_table = Provider {
            stream: None,
            ..record()
        };
        assert_eq!(chose(&no_table, "allow", &[], &json!({})), off);
        let acp = Provider {
            driver: DriverKind::Acp,
            ..record()
        };
        assert_eq!(chose(&acp, "allow", &[], &json!({})), off);
        let ask = Err("policy ask has no flags on the stream driver".to_owned());
        assert_eq!(chose(&record(), "ask", &[], &json!({})), ask);
        let not_adapter = Err("the command is not the agent's ACP adapter".to_owned());
        for cmd in [
            &["other-acp"][..],
            &["npx", "-y", "agent-acp", "--debug"],
            &["sh", "-c", "exec agent-acp"],
        ] {
            assert_eq!(
                chose(&record(), "allow", cmd, &json!({})),
                not_adapter,
                "{cmd:?}"
            );
        }
        let unmapped = |path: &str| {
            Err(format!(
                "--meta option {path} has no flag on the stream driver"
            ))
        };
        assert_eq!(
            chose(&record(), "allow", &[], &json!({"other": true})),
            unmapped("other")
        );
        let inner = json!({"opts": {"strict": true, "unknown": 1}});
        assert_eq!(
            chose(&record(), "allow", &[], &inner),
            unmapped("opts.unknown")
        );
    }

    #[test]
    fn a_served_lane_keeps_the_wrapper_and_maps_each_option_by_its_type() {
        let cmd = ["env", "K=V", "npx", "-y", "agent-acp@1.2"];
        let meta = json!({
            "system": {"append": "be brief"},
            "opts": {"sources": ["project", "local"], "strict": true, "loose": false,
                "settings": {"a": 1}, "turns": 3}
        });
        let want = [
            "env",
            "K=V",
            "agent",
            "-p",
            "--yes",
            "--append",
            "be brief",
            "--sources",
            "project,local",
            "--strict",
            "--settings",
            "{\"a\":1}",
            "--turns",
            "3",
        ];
        assert_eq!(chose(&record(), "allow", &cmd, &meta), Ok(words(&want)));
        let system = json!({"system": "terse"});
        let want = ["agent", "-p", "--yes", "--system", "terse"];
        assert_eq!(chose(&record(), "allow", &[], &system), Ok(words(&want)));
        let installed = words(&["/agents/agent-acp/0.1.0/agent-acp"]);
        let got = choose(&record(), "allow", &[], Some(&installed), None);
        assert_eq!(got, Ok(words(&["agent", "-p", "--yes"])));
    }
}
