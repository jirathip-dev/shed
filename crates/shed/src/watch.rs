use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::io::{self, BufRead as _, Read as _, Write as _};
use std::net::Shutdown;
use std::os::unix::net::UnixStream;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};

use anyhow::bail;
use rustix::termios::{isatty, tcgetwinsize};
use serde_json::Value;
use shed_server::Client;
use shed_server::proto::{
    Color, Event, EventKind, EventsParams, LsParams, Method, Run, ScreenUpdate, State,
    TerminalInfo, WatchParams,
};

use crate::attach::{self, RawMode};

const LINGER: Duration = Duration::from_mins(1);
const FRAME: Duration = Duration::from_millis(100);
const NARROW: usize = 80;
const LIST: usize = 34;
const ENTER: &str = "\x1b[?1049h\x1b[?25l\x1b[?7l\x1b[2J";
const LEAVE: &str = "\x1b[?7h\x1b[?25h\x1b[?1049l";

pub(crate) enum Msg {
    Event(Event),
    Screen(u64, ScreenUpdate),
    Closed,
}

pub(crate) trait Tty {
    fn input(&mut self, rx: &Receiver<Msg>) -> io::Result<(Vec<u8>, Vec<Msg>)>;
    fn now(&self) -> Instant;
    fn size(&self) -> (u16, u16);
    fn draw(&mut self, frame: &str) -> io::Result<()>;
    fn attach(&mut self, name: &str) -> anyhow::Result<()>;
}

pub(crate) fn watch(
    server: &crate::Server,
    labels: BTreeMap<String, String>,
) -> anyhow::Result<()> {
    if !isatty(io::stdin()) || !isatty(io::stdout()) {
        bail!("shed watch needs a terminal");
    }
    let connect = || server.connect(None);
    let mut tty = Screen {
        connect: &connect,
        _raw: RawMode::enter(0, 1)?,
    };
    tty.draw(ENTER)?;
    let machine = server.on.as_ref().map(|m| m.name.clone());
    run(&mut tty, &connect, machine, labels)
}

struct Screen<'a> {
    connect: &'a dyn Fn() -> anyhow::Result<Client>,
    _raw: RawMode,
}

impl Tty for Screen<'_> {
    fn input(&mut self, rx: &Receiver<Msg>) -> io::Result<(Vec<u8>, Vec<Msg>)> {
        let mut buf = [0u8; 256];
        let n = io::stdin().read(&mut buf)?;
        Ok((buf[..n].to_vec(), rx.try_iter().collect()))
    }

    fn now(&self) -> Instant {
        Instant::now()
    }

    fn size(&self) -> (u16, u16) {
        tcgetwinsize(io::stdout()).map_or((80, 24), |s| (s.ws_col, s.ws_row))
    }

    fn draw(&mut self, frame: &str) -> io::Result<()> {
        let mut out = io::stdout().lock();
        out.write_all(frame.as_bytes())?;
        out.flush()
    }

    fn attach(&mut self, name: &str) -> anyhow::Result<()> {
        self.draw(LEAVE)?;
        let done = (self.connect)().and_then(|client| attach::attach(client, name));
        self.draw(ENTER)?;
        done
    }
}

impl Drop for Screen<'_> {
    fn drop(&mut self) {
        let _ = self.draw(LEAVE);
    }
}

pub(crate) fn run(
    tty: &mut impl Tty,
    connect: &dyn Fn() -> anyhow::Result<Client>,
    machine: Option<String>,
    labels: BTreeMap<String, String>,
) -> anyhow::Result<()> {
    let (tx, rx) = mpsc::channel();
    let mut events = connect()?;
    let params = EventsParams {
        labels: labels.clone(),
        since: None,
    };
    let _: Value = events.call(Method::Events, &params)?;
    let to = tx.clone();
    std::thread::spawn(move || {
        let (reader, _writer) = events.split();
        for line in reader.lines().map_while(Result::ok) {
            if let Ok(event) = serde_json::from_str(&line)
                && to.send(Msg::Event(event)).is_err()
            {
                return;
            }
        }
        let _ = to.send(Msg::Closed);
    });
    let list: Vec<TerminalInfo> = connect()?.call(Method::Ls, &LsParams { labels })?;
    let mut w = Watch::new(machine, list, tty.now());
    let (mut watching, mut preview, mut n) = (None, None::<UnixStream>, 0);
    w.reselect();
    loop {
        if w.selected != watching {
            if let Some(old) = preview.take() {
                let _ = old.shutdown(Shutdown::Both);
            }
            n += 1;
            watching.clone_from(&w.selected);
            w.screen.clear();
            if let Some(name) = &watching {
                preview = watch_screen(connect, name, n, tx.clone()).ok();
            }
        }
        let (keys, msgs) = tty.input(&rx)?;
        let now = tty.now();
        for msg in msgs {
            match msg {
                Msg::Event(event) => w.event(event, now),
                Msg::Screen(k, update) if k == n => w.update(update),
                Msg::Screen(..) => {}
                Msg::Closed => bail!("the server closed the event stream"),
            }
        }
        match w.keys(&keys) {
            Some(Action::Quit) => return Ok(()),
            Some(Action::Attach(name)) => {
                w.status = tty
                    .attach(&name)
                    .err()
                    .map(|e| format!("{e:#}"))
                    .unwrap_or_default();
                (w.last_frame, w.dirty) = (None, true);
            }
            None => {}
        }
        w.tick(now);
        w.reselect();
        if let Some(frame) = w.frame(now, tty.size()) {
            tty.draw(&frame)?;
        }
    }
}

fn watch_screen(
    connect: &dyn Fn() -> anyhow::Result<Client>,
    name: &str,
    n: u64,
    tx: Sender<Msg>,
) -> anyhow::Result<UnixStream> {
    let mut client = connect()?;
    let params = WatchParams {
        terminal: name.to_owned(),
        since: None,
    };
    let _: Value = client.call(Method::ScreenWatch, &params)?;
    let (reader, writer) = client.split();
    std::thread::spawn(move || {
        for line in reader.lines().map_while(Result::ok) {
            if let Ok(update) = serde_json::from_str(&line)
                && tx.send(Msg::Screen(n, update)).is_err()
            {
                return;
            }
        }
    });
    Ok(writer)
}

#[derive(Debug, PartialEq, Eq)]
enum Action {
    Quit,
    Attach(String),
}

struct Row {
    labels: BTreeMap<String, String>,
    state: String,
    note: Option<String>,
    since: Instant,
    exited: Option<Instant>,
}

#[derive(Default)]
struct Watch {
    machine: Option<String>,
    rows: BTreeMap<String, Row>,
    selected: Option<String>,
    filter: String,
    filtering: bool,
    status: String,
    screen: Vec<Vec<Run>>,
    last_frame: Option<Instant>,
    dirty: bool,
}

impl Watch {
    fn new(machine: Option<String>, list: Vec<TerminalInfo>, now: Instant) -> Self {
        let rows = list.into_iter().map(|t| {
            let (state, note) = t
                .reported
                .map_or(("idle".to_owned(), None), |r| (r.state, r.note));
            let exited = (t.state == State::Exited).then_some(now);
            let row = Row {
                labels: t.labels,
                state,
                note,
                since: now,
                exited,
            };
            (t.name, row)
        });
        Self {
            machine,
            rows: rows.collect(),
            dirty: true,
            ..Self::default()
        }
    }

    fn event(&mut self, e: Event, now: Instant) {
        let fresh = || Row {
            labels: e.labels.clone(),
            state: "idle".to_owned(),
            note: None,
            since: now,
            exited: None,
        };
        match e.kind {
            EventKind::Started => drop(self.rows.insert(e.name, fresh())),
            EventKind::State(r) => {
                let row = self.rows.entry(e.name).or_insert_with(fresh);
                (row.state, row.note, row.since) = (r.state, r.note, now);
            }
            EventKind::Exited { .. } | EventKind::Killed => {
                if let Some(row) = self.rows.get_mut(&e.name) {
                    row.exited = Some(now);
                }
            }
            _ => return,
        }
        self.dirty = true;
    }

    fn update(&mut self, update: ScreenUpdate) {
        if update.full {
            self.screen = vec![Vec::new(); usize::from(update.rows)];
        }
        for line in update.lines {
            if let Some(row) = self.screen.get_mut(usize::from(line.row)) {
                row.clone_from(&line.runs);
            }
        }
        self.dirty = true;
    }

    fn tick(&mut self, now: Instant) {
        let before = self.rows.len();
        self.rows
            .retain(|_, r| r.exited.is_none_or(|t| now - t < LINGER));
        let stale = self
            .last_frame
            .is_none_or(|t| now - t >= Duration::from_secs(1));
        self.dirty |= stale || self.rows.len() != before;
    }

    fn visible(&self) -> Vec<(&str, &str)> {
        let mut list: Vec<(&str, &str)> = self
            .rows
            .iter()
            .filter(|(name, row)| {
                name.contains(&self.filter)
                    || row
                        .labels
                        .iter()
                        .any(|(k, v)| format!("{k}={v}").contains(&self.filter))
            })
            .map(|(name, row)| {
                let fleet = row.labels.get("fleet").map_or("", String::as_str);
                (fleet, name.as_str())
            })
            .collect();
        list.sort_unstable();
        list
    }

    fn reselect(&mut self) {
        let visible = self.visible();
        if !visible
            .iter()
            .any(|(_, n)| self.selected.as_deref() == Some(n))
        {
            self.selected = visible.first().map(|(_, n)| (*n).to_owned());
            self.dirty = true;
        }
    }

    fn step(&mut self, down: bool) {
        let visible = self.visible();
        let at = visible
            .iter()
            .position(|(_, n)| self.selected.as_deref() == Some(n));
        let to = match at {
            Some(i) if down => (i + 1).min(visible.len().saturating_sub(1)),
            Some(i) => i.saturating_sub(1),
            None => 0,
        };
        self.selected = visible.get(to).map(|(_, n)| (*n).to_owned());
        self.dirty = true;
    }

    fn keys(&mut self, mut bytes: &[u8]) -> Option<Action> {
        if !bytes.is_empty() {
            self.status.clear();
            self.dirty = true;
        }
        while let [first, rest @ ..] = bytes {
            let arrow = match bytes {
                [0x1b, b'[' | b'O', b'A', ..] => Some(false),
                [0x1b, b'[' | b'O', b'B', ..] => Some(true),
                _ => None,
            };
            bytes = if arrow.is_some() { &bytes[3..] } else { rest };
            if self.filtering {
                match first {
                    _ if arrow.is_some() => {}
                    b'\r' | b'\n' => self.filtering = false,
                    0x1b => (self.filtering, self.filter) = (false, String::new()),
                    0x7f | 0x08 => drop(self.filter.pop()),
                    c if c.is_ascii_graphic() || *c == b' ' => self.filter.push(char::from(*c)),
                    _ => {}
                }
                continue;
            }
            match (arrow, first) {
                (Some(down), _) => self.step(down),
                (None, b'j') => self.step(true),
                (None, b'k') => self.step(false),
                (None, b'/') => self.filtering = true,
                (None, b'\r' | b'\n') => return self.selected.clone().map(Action::Attach),
                (None, b'q' | 0x03) => return Some(Action::Quit),
                _ => {}
            }
        }
        None
    }

    fn list(&self, now: Instant, width: usize) -> Vec<(String, bool)> {
        let mut lines = Vec::new();
        if let Some(machine) = &self.machine {
            lines.push((format!("\x1b[1m{}\x1b[0m", clip(machine, width)), false));
        }
        let mut group = None;
        for (fleet, name) in self.visible() {
            if group != Some(fleet) {
                group = Some(fleet);
                let head = if fleet.is_empty() {
                    "(no fleet)"
                } else {
                    fleet
                };
                lines.push((format!("\x1b[1;4m{}\x1b[0m", clip(head, width)), false));
            }
            let row = &self.rows[name];
            let (color, dot) = match row.state.as_str() {
                "working" => (33, '●'),
                "blocked" => (35, '●'),
                "done" => (32, '●'),
                "error" => (31, '●'),
                _ => (90, '○'),
            };
            let issue = row.labels.get("issue").map(|i| format!("{fleet}#{i}"));
            let role = row.labels.get("role").cloned();
            let mut text = format!(" {name}");
            for part in [issue, role, Some(age(now - row.since))]
                .into_iter()
                .flatten()
            {
                let _ = write!(text, " {part}");
            }
            let selected = self.selected.as_deref() == Some(name);
            let dim = if row.exited.is_some() { ";2" } else { "" };
            let inverse = if selected { ";7" } else { "" };
            let text = clip(&text, width.saturating_sub(1));
            lines.push((
                format!("\x1b[0{dim}{inverse}m\x1b[{color}m{dot}\x1b[39m{text}\x1b[0m"),
                selected,
            ));
        }
        lines
    }

    fn frame(&mut self, now: Instant, (cols, rows): (u16, u16)) -> Option<String> {
        if !self.dirty || self.last_frame.is_some_and(|t| now - t < FRAME) {
            return None;
        }
        (self.dirty, self.last_frame) = (false, Some(now));
        let (cols, rows) = (usize::from(cols), usize::from(rows.max(2)));
        let wide = cols >= NARROW;
        let width = if wide { LIST } else { cols };
        let list = self.list(now, width);
        let body = rows - 1;
        let at = list.iter().position(|(_, s)| *s).unwrap_or(0);
        let skip = (at + 1).saturating_sub(body);
        let blank = " ".repeat(width);
        let mut out = String::new();
        for r in 0..body {
            let left = list.get(skip + r).map_or(blank.as_str(), |(l, _)| l);
            let _ = write!(out, "\x1b[{};1H{left}", r + 1);
            if wide {
                out.push_str("\x1b[0;90m│\x1b[0m");
                let pane = cols - width - 1;
                if r == 0 {
                    out.push_str(&self.header(pane));
                } else if let Some(runs) = self.screen.get(r - 1) {
                    out.push_str(&draw_runs(runs, pane));
                }
            }
            out.push_str("\x1b[0m\x1b[K");
        }
        let footer = if self.filtering {
            format!("/{}", self.filter)
        } else if !self.status.is_empty() {
            self.status.clone()
        } else {
            let filter = if self.filter.is_empty() {
                String::new()
            } else {
                format!("  [/{}]", self.filter)
            };
            format!("↑↓ select  enter attach (ctrl+\\ back)  / filter  q quit{filter}")
        };
        let _ = write!(out, "\x1b[{rows};1H\x1b[0;2m{}\x1b[0m", clip(&footer, cols));
        Some(out)
    }

    fn header(&self, width: usize) -> String {
        let Some((name, row)) = self
            .selected
            .as_ref()
            .and_then(|n| self.rows.get_key_value(n))
        else {
            return String::new();
        };
        let state = if row.exited.is_some() {
            "exited"
        } else {
            &row.state
        };
        let mut head = format!("{name} · {state}");
        if let Some(note) = &row.note {
            let _ = write!(head, " · {note}");
        }
        format!("\x1b[1m{}", clip(&head, width))
    }
}

fn clip(s: &str, width: usize) -> String {
    format!("{:width$}", s.chars().take(width).collect::<String>())
}

fn draw_runs(runs: &[Run], mut width: usize) -> String {
    let mut out = String::new();
    for run in runs {
        let text: String = run.text.chars().take(width).collect();
        width -= text.chars().count();
        let mut sgr = String::from("0");
        let flags = [
            (run.bold, 1),
            (run.italic, 3),
            (run.underline, 4),
            (run.inverse, 7),
        ];
        for (_, code) in flags.into_iter().filter(|(on, _)| *on) {
            let _ = write!(sgr, ";{code}");
        }
        for (color, base) in [(run.fg, 38), (run.bg, 48)] {
            let _ = match color {
                Some(Color::Index(i)) => write!(sgr, ";{base};5;{i}"),
                Some(Color::Rgb([r, g, b])) => write!(sgr, ";{base};2;{r};{g};{b}"),
                None => Ok(()),
            };
        }
        let _ = write!(out, "\x1b[{sgr}m{text}");
    }
    out
}

fn age(d: Duration) -> String {
    match d.as_secs() {
        s @ 0..60 => format!("{s}s"),
        s @ 60..3600 => format!("{}m", s / 60),
        s => format!("{}h", s / 3600),
    }
}

#[cfg(test)]
mod tests {
    use std::io::BufReader;
    use std::sync::{Arc, Mutex};

    use serde_json::json;
    use shed_server::proto::{Hello, Request, Response};

    use super::*;

    #[derive(Default)]
    struct Fake {
        list: Vec<Value>,
        events: Vec<Value>,
        screens: BTreeMap<String, Vec<Value>>,
        calls: Mutex<Vec<String>>,
    }

    impl Fake {
        fn connect(self: &Arc<Self>) -> anyhow::Result<Client> {
            let (ours, theirs) = UnixStream::pair()?;
            let fake = Arc::clone(self);
            std::thread::spawn(move || fake.serve(theirs));
            Ok(Client::over(ours)?)
        }

        fn serve(&self, stream: UnixStream) {
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut out = stream;
            let mut line = String::new();
            while reader.read_line(&mut line).unwrap_or(0) > 0 {
                let req: Request = serde_json::from_str(&line).unwrap();
                line.clear();
                let terminal = req.params.get("terminal").and_then(Value::as_str);
                let call = format!("{} {}", req.method, terminal.unwrap_or(""));
                self.calls.lock().unwrap().push(call.trim_end().to_owned());
                let result = match req.method.as_str() {
                    "hello" => serde_json::to_value(Hello::ours()).unwrap(),
                    "ls" => Value::from(self.list.clone()),
                    _ => json!({}),
                };
                let mut reply = serde_json::to_vec(&Response::ok(req.id.unwrap(), result)).unwrap();
                reply.push(b'\n');
                let stream = match req.method.as_str() {
                    "events" => self.events.clone(),
                    "screen.watch" => self
                        .screens
                        .get(terminal.unwrap())
                        .cloned()
                        .unwrap_or_default(),
                    _ => Vec::new(),
                };
                for value in stream {
                    reply.extend(serde_json::to_vec(&value).unwrap());
                    reply.push(b'\n');
                }
                if out.write_all(&reply).is_err() {
                    return;
                }
            }
        }
    }

    struct Step {
        keys: &'static [u8],
        msgs: usize,
        ms: u64,
    }

    const fn step(keys: &'static [u8], msgs: usize, ms: u64) -> Step {
        Step { keys, msgs, ms }
    }

    struct FakeTty {
        start: Instant,
        clock: Duration,
        steps: std::vec::IntoIter<Step>,
        size: (u16, u16),
        frames: Vec<(Duration, String)>,
        attached: Vec<String>,
    }

    impl Tty for FakeTty {
        fn input(&mut self, rx: &Receiver<Msg>) -> io::Result<(Vec<u8>, Vec<Msg>)> {
            let Some(step) = self.steps.next() else {
                return Ok((b"q".to_vec(), Vec::new()));
            };
            self.clock += Duration::from_millis(step.ms);
            let wait = || rx.recv_timeout(Duration::from_secs(10)).expect("a message");
            Ok((step.keys.to_vec(), (0..step.msgs).map(|_| wait()).collect()))
        }

        fn now(&self) -> Instant {
            self.start + self.clock
        }

        fn size(&self) -> (u16, u16) {
            self.size
        }

        fn draw(&mut self, frame: &str) -> io::Result<()> {
            self.frames.push((self.clock, frame.to_owned()));
            Ok(())
        }

        fn attach(&mut self, name: &str) -> anyhow::Result<()> {
            self.attached.push(name.to_owned());
            Ok(())
        }
    }

    fn watch(fake: Fake, size: (u16, u16), steps: Vec<Step>) -> (FakeTty, Arc<Fake>) {
        let fake = Arc::new(fake);
        let mut tty = FakeTty {
            start: Instant::now(),
            clock: Duration::ZERO,
            steps: steps.into_iter(),
            size,
            frames: Vec::new(),
            attached: Vec::new(),
        };
        let connect = || fake.connect();
        run(&mut tty, &connect, None, BTreeMap::new()).unwrap();
        (tty, fake)
    }

    fn terminal(name: &str, state: &str, reported: Option<&str>, labels: &Value) -> Value {
        let mut t = json!({"name": name, "labels": labels, "cwd": "/", "cmd": ["sh"], "keep": false,
            "state": state, "exit": null, "pid": null});
        if let Some(r) = reported {
            t["reported"] = json!({"state": r});
        }
        t
    }

    fn event(seq: u64, name: &str, kind: &Value) -> Value {
        let mut e = json!({"seq": seq, "name": name, "labels": {"fleet": "shed"}});
        e.as_object_mut()
            .unwrap()
            .extend(kind.as_object().unwrap().clone());
        e
    }

    fn screen(full: bool, rows: &[(u16, &str)]) -> Value {
        let lines: Vec<Value> = rows
            .iter()
            .map(|(row, text)| {
                json!({"row": row, "runs": [{"text": text, "fg": 2, "bg": null,
                "bold": false, "italic": false, "underline": false, "inverse": false}]})
            })
            .collect();
        json!({"seq": 1, "full": full, "rows": 3, "cols": 100, "cursor": [0, 0], "scrollback": 0,
            "lines": lines})
    }

    fn plain(frame: &str) -> String {
        let mut out = String::new();
        let mut chars = frame.chars();
        while let Some(c) = chars.next() {
            if c == '\x1b' {
                chars.by_ref().find(char::is_ascii_alphabetic);
            } else {
                out.push(c);
            }
        }
        out
    }

    #[test]
    fn the_list_follows_the_event_stream() {
        let fake = Fake {
            list: vec![terminal(
                "a",
                "running",
                Some("working"),
                &json!({"fleet": "shed", "issue": "1", "role": "impl"}),
            )],
            events: vec![
                event(1, "b", &json!({"event": "started"})),
                event(
                    2,
                    "b",
                    &json!({"event": "state", "state": "blocked", "note": "which?"}),
                ),
                event(3, "a", &json!({"event": "exited", "code": 0})),
            ],
            ..Fake::default()
        };
        let steps = vec![
            step(b"", 0, 0),
            step(b"", 1, 200),
            step(b"", 1, 200),
            step(b"", 1, 200),
            step(b"", 0, 59_000),
            step(b"", 0, 1_000),
        ];
        let (tty, _) = watch(fake, (100, 10), steps);
        let frames: Vec<&str> = tty.frames.iter().map(|(_, f)| f.as_str()).collect();
        assert_eq!(frames.len(), 6, "one frame a step");
        assert!(frames[0].contains("\x1b[0;7m\x1b[33m●\x1b[39m a shed#1 impl 0s"));
        assert!(
            plain(frames[0]).contains("a · working"),
            "the preview's header"
        );
        assert!(
            frames[1].contains("\x1b[0m\x1b[90m○\x1b[39m b 0s"),
            "b started, idle"
        );
        assert!(
            frames[2].contains("\x1b[0m\x1b[35m●\x1b[39m b 0s"),
            "b blocked"
        );
        assert!(
            frames[3].contains("\x1b[0;2;7m\x1b[33m●\x1b[39m a"),
            "a exited, dim"
        );
        assert!(plain(frames[3]).contains("a · exited"));
        assert!(
            plain(frames[4]).contains(" a shed#1 impl"),
            "still there after 59 s"
        );
        assert!(!plain(frames[5]).contains(" a shed#1"), "gone after 60 s");
        assert!(
            plain(frames[5]).contains("b · blocked · which?"),
            "the selection moved to b"
        );
    }

    #[test]
    fn the_preview_shows_the_screen_at_most_ten_times_a_second() {
        let mut updates = vec![screen(true, &[(0, "hello"), (2, &"x".repeat(100))])];
        updates.extend((1..=9).map(|i| screen(false, &[(1, &format!("n={i}"))])));
        let fake = Fake {
            list: vec![terminal("a", "running", None, &json!({}))],
            screens: BTreeMap::from([("a".to_owned(), updates)]),
            ..Fake::default()
        };
        let mut steps = vec![step(b"", 1, 0)];
        steps.extend((0..9).map(|_| step(b"", 1, 20)));
        steps.push(step(b"", 0, 100));
        let (tty, fake) = watch(fake, (80, 6), steps);
        let times: Vec<u128> = tty.frames.iter().map(|(t, _)| t.as_millis()).collect();
        assert_eq!(times, [0, 100, 280], "10 updates in 180 ms, 3 frames");
        let first = plain(&tty.frames[0].1);
        assert!(first.contains("│hello"));
        assert!(
            first.contains(&format!("│{}", "x".repeat(45))),
            "clipped at the pane"
        );
        assert!(!first.contains(&"x".repeat(46)), "never wrapped");
        assert!(
            plain(&tty.frames[2].1).contains("│n=9"),
            "the latest screen"
        );
        assert_eq!(
            *fake.calls.lock().unwrap(),
            ["hello", "events", "hello", "ls", "hello", "screen.watch a"]
        );
    }

    #[test]
    fn enter_attaches_and_ctrl_backslash_comes_back_to_the_selection() {
        let fake = Fake {
            list: vec![
                terminal("a", "running", None, &json!({"role": "impl"})),
                terminal("b", "running", None, &json!({"role": "rev"})),
            ],
            ..Fake::default()
        };
        let steps = vec![
            step(b"\x1b[B", 0, 100),
            step(b"\r", 0, 100),
            step(b"", 0, 100),
            step(b"/impl\r", 0, 100),
            step(b"\r", 0, 100),
        ];
        let (tty, fake) = watch(fake, (100, 10), steps);
        assert_eq!(tty.attached, ["b", "a"]);
        let back = &tty.frames[1].1;
        assert!(
            back.contains("\x1b[0;7m\x1b[90m○\x1b[39m b rev"),
            "b still selected"
        );
        let filtered = plain(&tty.frames[2].1);
        assert!(
            !filtered.contains(" b rev") && filtered.contains("[/impl]"),
            "the filter hides b"
        );
        let calls = fake.calls.lock().unwrap();
        assert!(
            calls.contains(&"screen.watch b".to_owned()),
            "the preview follows the selection"
        );
        let input = [
            "send",
            "keys",
            "resize",
            "terminal.input",
            "terminal.keys",
            "terminal.resize",
        ];
        assert!(
            !calls.iter().any(|c| input.contains(&c.as_str())),
            "no input to any terminal"
        );
    }

    #[test]
    fn a_narrow_terminal_hides_the_preview() {
        let fake = Fake {
            list: vec![terminal("a", "running", None, &json!({}))],
            screens: BTreeMap::from([("a".to_owned(), vec![screen(true, &[(0, "hello")])])]),
            ..Fake::default()
        };
        let (tty, _) = watch(fake, (79, 6), vec![step(b"", 1, 0), step(b"\r", 0, 100)]);
        let frame = plain(&tty.frames[0].1);
        assert!(frame.contains(" a 0s"));
        assert!(!frame.contains('│') && !frame.contains("hello"));
        assert_eq!(tty.attached, ["a"]);
    }
}
