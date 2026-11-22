use std::collections::BTreeMap;
use std::io;
use std::path::Path;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use shed_script::HostError;
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader, Lines};
use tokio::process::{ChildStdin, ChildStdout};
use tokio::sync::Mutex;

use crate::proto::{Caller, Method, Route};
use crate::runner;

const REASON: usize = 400;

#[derive(Deserialize)]
struct File {
    script: Option<Config>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Config {
    guard: Vec<String>,
    #[serde(default = "ten")]
    guard_timeout: f64,
    #[serde(default)]
    guard_per: Per,
}

fn ten() -> f64 {
    10.0
}

#[derive(Deserialize, Default, Clone, Copy)]
#[serde(rename_all = "lowercase")]
enum Per {
    #[default]
    Run,
    Call,
}

#[derive(Serialize)]
struct Asked<'a> {
    id: u64,
    method: &'a str,
    params: &'a Value,
    machine: Option<&'a str>,
    caller: Option<&'a Caller>,
}

#[derive(Deserialize)]
struct Answer {
    id: u64,
    allow: bool,
    #[serde(default)]
    reason: String,
}

pub(super) struct Guard {
    config: Result<Option<(Config, Duration)>, String>,
    held: Mutex<Held>,
}

#[derive(Default)]
struct Held {
    asked: u64,
    alive: Option<Alive>,
}

struct Alive {
    stdin: ChildStdin,
    lines: Lines<BufReader<ChildStdout>>,
    _process: runner::held::Held,
}

type Refusal = (&'static str, String);

impl Guard {
    pub(super) fn for_run(path: &Path) -> Self {
        Self {
            config: read(path),
            held: Mutex::default(),
        }
    }

    pub(super) async fn ask(
        &self,
        m: Method,
        params: &Value,
        caller: Option<&Caller>,
    ) -> Result<(), HostError> {
        let refused = |(code, why): Refusal| HostError {
            name: "GuardError".into(),
            code: code.into(),
            message: format!("{} {why}", m.name()),
        };
        let (config, timeout) = match &self.config {
            Ok(Some(config)) => config,
            Ok(None) => {
                let why =
                    "refused: this server has no script guard ([script] guard in config.toml)";
                return Err(refused(("NO_GUARD", why.into())));
            }
            Err(e) => return Err(refused(("GUARD_FAILED", format!("refused: {e}")))),
        };
        let mut held = self.held.lock().await;
        held.asked += 1;
        let asked = Asked {
            id: held.asked,
            method: m.name(),
            params,
            machine: acts_on(m, params),
            caller,
        };
        let mut line = serde_json::to_vec(&asked).unwrap_or_default();
        line.push(b'\n');
        let answer = match config.guard_per {
            Per::Run => held.exchange(&config.guard, line, *timeout).await,
            Per::Call => once(&config.guard, line, *timeout).await,
        };
        answer.map_err(refused)
    }
}

impl Held {
    async fn exchange(
        &mut self,
        guard: &[String],
        line: Vec<u8>,
        timeout: Duration,
    ) -> Result<(), Refusal> {
        let id = self.asked;
        let asked = tokio::time::timeout(timeout, async {
            let alive = match &mut self.alive {
                Some(alive) => alive,
                None => self.alive.insert(start(guard)?),
            };
            let ended = || failed("ended before it answered");
            alive.stdin.write_all(&line).await.map_err(|_| ended())?;
            let answer = alive.lines.next_line().await;
            let answer = answer.ok().flatten().ok_or_else(ended)?;
            match serde_json::from_str::<Answer>(&answer) {
                Ok(a) if a.id == id => Ok(a),
                _ => Err(failed(&format!("answered out of turn: {}", cut(&answer)))),
            }
        });
        let answer = match asked.await {
            Ok(Ok(answer)) => answer,
            Ok(Err(refusal)) => {
                self.alive = None;
                return Err(refusal);
            }
            Err(_) => {
                self.alive = None;
                return Err(late(timeout));
            }
        };
        let why = format!("refused by the guard: {}", cut(&answer.reason));
        answer.allow.then_some(()).ok_or(("GUARD_REFUSED", why))
    }
}

fn start(guard: &[String]) -> Result<Alive, Refusal> {
    let started = runner::held::held(guard, Path::new("."), &BTreeMap::new(), false);
    let mut process = started.map_err(|e| failed(&format!("could not start: {e}")))?;
    let pipes = process.child.stdin.take().zip(process.child.stdout.take());
    let (stdin, stdout) = pipes.ok_or_else(|| failed("could not start: no pipes"))?;
    Ok(Alive {
        stdin,
        lines: BufReader::new(stdout).lines(),
        _process: process,
    })
}

async fn once(guard: &[String], line: Vec<u8>, timeout: Duration) -> Result<(), Refusal> {
    let none = BTreeMap::new();
    let ran = runner::exec(guard, Path::new("."), &none, line, timeout, 64 << 10).await;
    let (status, stdout, stderr) = match ran {
        Err(e) if e.kind() == io::ErrorKind::TimedOut => return Err(late(timeout)),
        Err(e) => return Err(failed(&format!("could not start: {e}"))),
        Ok(ran) => ran,
    };
    if !stdout.is_empty() {
        let why = "wrote to stdout; a guard asked per call answers by its exit status (guard_per in [script])";
        return Err(failed(why));
    }
    if status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&stderr);
    let last = stderr.lines().rev().find(|l| !l.trim().is_empty());
    let code = status
        .code()
        .map_or_else(|| "a signal".into(), |c| c.to_string());
    let reason = last.map_or_else(|| format!("exit {code}"), cut);
    Err(("GUARD_REFUSED", format!("refused by the guard: {reason}")))
}

fn failed(why: &str) -> Refusal {
    ("GUARD_FAILED", format!("refused: the guard {why}"))
}

fn late(timeout: Duration) -> Refusal {
    let secs = timeout.as_secs_f64();
    let why = format!("refused: the guard did not answer within {secs} s");
    ("GUARD_TIMEOUT", why)
}

fn cut(text: &str) -> String {
    text.chars().take(REASON).collect()
}

fn acts_on(m: Method, params: &Value) -> Option<&str> {
    let field = |k: &str| params.get(k).and_then(Value::as_str);
    match m.route()? {
        Route::Machine => field("machine"),
        Route::Name => field("name")?.split_once('/').map(|(machine, _)| machine),
    }
}

fn read(path: &Path) -> Result<Option<(Config, Duration)>, String> {
    let text = match std::fs::read_to_string(path) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        text => text.map_err(|e| format!("{}: {e}", path.display()))?,
    };
    let bad = |e: &dyn std::fmt::Display| format!("[script] in {}: {e}", path.display());
    let file: File = toml::from_str(&text).map_err(|e| bad(&e))?;
    let Some(config) = file.script else {
        return Ok(None);
    };
    if config.guard.is_empty() {
        return Err(bad(&"guard is empty"));
    }
    let timeout = Duration::try_from_secs_f64(config.guard_timeout).map_err(|e| bad(&e))?;
    Ok(Some((config, timeout)))
}
