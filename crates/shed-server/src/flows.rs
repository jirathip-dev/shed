use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Debug;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use serde::Serialize;
use shed_accounts::Store;
use shed_accounts::keys::Keys;
use shed_core::proctree::{self, ProcessTable as _};
use shed_proto::flow::{
    Differs, Disagreement, FlowReport, Mode, Step, Table, Tracker, account::Account, drain::Drain,
    key::Key, link::Link, machine::Machine, terminal::Terminal,
};
use shed_proto::{BUILD, LsParams, Method, State, TerminalInfo};

use crate::call::blocking;
use crate::links::Links;
use crate::{Shared, lock, machines};

mod account;
mod key;
mod link;
mod machine;
mod shadow;
mod terminal;

pub(crate) use shadow::report;

const MODE: Mode = Mode::Shadow;

pub(crate) struct Seen<T: Table> {
    pub(crate) id: String,
    pub(crate) seen: String,
    pub(crate) facts: Vec<T::Fact>,
    pub(crate) code: &'static str,
    pub(crate) agrees: &'static [T::State],
}

pub(crate) trait Shadow: Table {
    const GONE: Option<Self::Fact>;
    const UNDONE: &'static [Self::Action];
}

impl Shadow for Drain {
    const GONE: Option<Self::Fact> = None;
    const UNDONE: &'static [Self::Action] = &[];
}

struct Out {
    now: u64,
    lines: Vec<Disagreement>,
}

struct Subject<T: Table> {
    tracker: Tracker<T>,
    code: &'static str,
    agrees: &'static [T::State],
    fact: String,
    differs: Option<u64>,
    told: bool,
}

impl<T: Table> Subject<T> {
    fn line(&self, id: &str, kind: Differs, row: &str, table: String, now: u64) -> Disagreement {
        Disagreement {
            at: now,
            build: BUILD.to_owned(),
            flow: T::NAME,
            id: id.to_owned(),
            kind,
            row: row.to_owned(),
            fact: self.fact.clone(),
            table,
            code: self.code.to_owned(),
        }
    }
}

fn name(debug: impl Debug, sep: char) -> String {
    let mut out = String::new();
    for c in format!("{debug:?}").chars() {
        if c.is_ascii_uppercase() && !out.is_empty() {
            out.push(sep);
        }
        out.push(c.to_ascii_lowercase());
    }
    out
}

fn wire(state: impl Serialize) -> String {
    let value = serde_json::to_value(state).ok();
    value
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_default()
}

fn record<T: Shadow>(
    report: &mut FlowReport,
    id: &str,
    s: &Subject<T>,
    step: &Step<T>,
    tell: bool,
    out: &mut Out,
) {
    let count = |map: &mut BTreeMap<String, u64>, key: &str| {
        *map.entry(key.to_owned()).or_default() += 1;
    };
    count(&mut report.fired, step.row.id);
    if step.unexpected {
        count(&mut report.unexpected, step.row.id);
    }
    if let Some(why) = step.stuck {
        count(&mut report.stuck, why);
    }
    report.never_seen.retain(|f| *f != s.fact);
    if let Some(act) = step.run.filter(|a| tell && T::UNDONE.contains(a)) {
        let line = s.line(id, Differs::Action, step.row.id, name(act, '-'), out.now);
        out.lines.push(line);
    }
}

struct Shadowed<T: Shadow> {
    subjects: BTreeMap<String, Subject<T>>,
    seen: BTreeMap<String, String>,
    report: FlowReport,
}

impl<T: Shadow> Shadowed<T> {
    fn new(kept: &[FlowReport]) -> Self {
        let fresh = || FlowReport {
            flow: T::NAME,
            mode: MODE,
            subjects: 0,
            fired: BTreeMap::new(),
            never_seen: (T::FACTS.iter().filter(|f| **f != T::GAVE_UP))
                .map(|f| name(f, '_'))
                .collect(),
            unexpected: BTreeMap::new(),
            stuck: BTreeMap::new(),
            disagreements: Vec::new(),
        };
        let report = kept.iter().find(|r| r.flow == T::NAME).cloned();
        let report = report.unwrap_or_else(fresh);
        Self {
            subjects: BTreeMap::new(),
            seen: BTreeMap::new(),
            report: FlowReport {
                mode: MODE,
                ..report
            },
        }
    }

    fn step(&mut self, id: &str, s: &mut Subject<T>, fact: T::Fact, tell: bool, out: &mut Out) {
        let step = s.tracker.on(fact, out.now);
        s.fact = name(fact, '_');
        record(&mut self.report, id, s, &step, tell, out);
    }

    fn observe(&mut self, all: Vec<Seen<T>>, out: &mut Out) {
        let ids: BTreeSet<&str> = all.iter().map(|s| s.id.as_str()).collect();
        let gone = self.seen.keys().filter(|k| !ids.contains(k.as_str()));
        for id in gone.cloned().collect::<Vec<_>>() {
            self.seen.remove(&id);
            let (Some(mut s), Some(fact)) = (self.subjects.remove(&id), T::GONE) else {
                continue;
            };
            s.code = "none";
            self.step(&id, &mut s, fact, true, out);
            let state = s.tracker.state();
            if state != T::START {
                let row = s.tracker.kept().row;
                out.lines
                    .push(s.line(&id, Differs::State, &row, wire(state), out.now));
            }
        }
        for seen in all {
            if self.seen.get(&seen.id) == Some(&seen.seen) {
                continue;
            }
            let old = self.subjects.remove(&seen.id);
            let tell = old.is_some();
            let mut s = old.unwrap_or_else(|| Subject {
                tracker: Tracker::new(out.now),
                code: "",
                agrees: &[],
                fact: String::new(),
                differs: None,
                told: false,
            });
            (s.code, s.agrees) = (seen.code, seen.agrees);
            for fact in seen.facts {
                self.step(&seen.id, &mut s, fact, tell, out);
            }
            self.subjects.insert(seen.id.clone(), s);
            self.seen.insert(seen.id, seen.seen);
        }
    }

    fn tick(&mut self, out: &mut Out) {
        let now = out.now;
        for (id, s) in &mut self.subjects {
            if let Some(step) = s.tracker.tick(now).filter(|st| st.stuck.is_some()) {
                s.fact = name(T::GAVE_UP, '_');
                record(&mut self.report, id, s, &step, true, out);
            }
            let state = s.tracker.state();
            if s.agrees.contains(&state) {
                (s.differs, s.told) = (None, false);
                continue;
            }
            let since = *s.differs.get_or_insert(now);
            if !s.told && now > since + 2 {
                s.told = true;
                let row = s.tracker.kept().row;
                out.lines
                    .push(s.line(id, Differs::State, &row, wire(state), now));
            }
        }
        self.report.subjects = u64::try_from(self.subjects.len()).unwrap_or(u64::MAX);
    }
}

struct Runner {
    state: PathBuf,
    terminal: Shadowed<Terminal>,
    link: Shadowed<Link>,
    machine: Shadowed<Machine>,
    drain: Shadowed<Drain>,
    account: Shadowed<Account>,
    key: Shadowed<Key>,
}

impl Runner {
    fn new(state: &Path) -> Self {
        let kept = shadow::counts(state);
        Self {
            state: state.to_owned(),
            terminal: Shadowed::new(&kept),
            link: Shadowed::new(&kept),
            machine: Shadowed::new(&kept),
            drain: Shadowed::new(&kept),
            account: Shadowed::new(&kept),
            key: Shadowed::new(&kept),
        }
    }

    fn reports(&self) -> Vec<FlowReport> {
        [
            &self.terminal.report,
            &self.link.report,
            &self.machine.report,
            &self.drain.report,
            &self.account.report,
            &self.key.report,
        ]
        .map(FlowReport::clone)
        .into()
    }

    fn tick(&mut self, out: &mut Out) {
        self.terminal.tick(out);
        self.link.tick(out);
        self.machine.tick(out);
        self.drain.tick(out);
        self.account.tick(out);
        self.key.tick(out);
    }

    async fn often(&mut self, registry: &Shared, links: &Links, out: &mut Out) {
        let (infos, leaders) = {
            let reg = lock(registry);
            (reg.ls(&LsParams::default()), reg.leaders())
        };
        let procs = if infos.iter().any(|t| t.state == State::Exited) {
            blocking(|| proctree::system().list().ok())
                .await
                .ok()
                .flatten()
        } else {
            None
        };
        let seen = terminal::observe(&infos, &leaders, procs.as_deref(), &self.terminal.seen);
        self.terminal.observe(seen, out);
        let report = links.report();
        self.link
            .observe(link::observe(&report, &self.link.seen), out);
    }

    async fn seldom(&mut self, registry: &Shared, links: &Links, out: &mut Out) {
        let (config, state) = (links.config.clone(), self.state.clone());
        let dir = lock(registry).providers.clone();
        let read = move || {
            let store = Store::beside(&state);
            let records = crate::accounts::records_in(&dir).into_iter();
            let all =
                records.flat_map(|r| store.ls(&r).into_iter().map(move |a| (r.id.clone(), a)));
            let accounts: Vec<_> = all.collect();
            (
                machines::load(&config).ok(),
                accounts,
                Keys::beside(&state).ls(),
            )
        };
        let Ok((listed, accounts, keys)) = blocking(read).await else {
            return;
        };
        if let Some(listed) = listed {
            let seen = machine::observe(&listed, &links.report(), &self.machine.seen);
            self.machine.observe(seen, out);
        }
        let accounts: Vec<_> = {
            let reg = lock(registry);
            let runs = |(id, a): (String, shed_proto::Account)| (reg.runs(&id, &a.name), a);
            accounts.into_iter().map(runs).collect()
        };
        self.account.observe(account::observe(&accounts), out);
        let mut there = BTreeMap::new();
        for m in keys
            .iter()
            .filter_map(|k| Some(k.terminal.split_once('/')?.0))
        {
            if there.contains_key(m) {
                continue;
            }
            let ls = links.call(m, Method::Ls, serde_json::json!({}));
            let ls = tokio::time::timeout(Duration::from_secs(10), ls).await;
            let ls = ls.ok().and_then(Result::ok);
            let ls = ls.and_then(|v| serde_json::from_value::<Vec<TerminalInfo>>(v).ok());
            there.insert(m.to_owned(), ls);
        }
        self.key.observe(key::observe(&keys, &there), out);
    }
}

pub(crate) async fn run(registry: Shared, links: Arc<Links>) {
    let state = lock(&registry).path.clone();
    let mut runner = Runner::new(&state);
    for n in 0u64.. {
        tokio::time::sleep(Duration::from_secs(1)).await;
        let now = shed_core::clock::unix_now();
        let mut out = Out {
            now,
            lines: Vec::new(),
        };
        if n % 2 == 0 {
            runner.often(&registry, &links, &mut out).await;
        }
        if n % 60 == 0 {
            runner.seldom(&registry, &links, &mut out).await;
        }
        runner.tick(&mut out);
        let counts = (n % 60 == 0).then(|| runner.reports());
        let (state, lines) = (state.clone(), out.lines);
        blocking(move || shadow::write(&state, &lines, counts.as_deref()).ok())
            .await
            .ok();
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::time::Duration;

    use serde_json::json;
    use shed_core::proctree::{self, ProcessTable as _};
    use shed_proto::flow::{Differs, FlowReport};
    use shed_proto::{KeyState, LinkState, LsParams, ProxyKey};

    use super::*;
    use crate::registry::Registry;

    fn out(now: u64) -> Out {
        Out {
            now,
            lines: Vec::new(),
        }
    }

    fn flow<'a>(reports: &'a [FlowReport], name: shed_proto::flow::FlowName) -> &'a FlowReport {
        reports.iter().find(|r| r.flow == name).unwrap()
    }

    fn group_alive(pgid: i32) -> bool {
        let procs = proctree::system().list().unwrap();
        procs.iter().any(|p| p.pgid == pgid && !p.zombie)
    }

    #[tokio::test]
    async fn a_program_that_leaves_a_child_is_one_end_tree_line_then_stuck_then_gone() {
        let state = crate::temp_dir("flows-terminal").join("state.json");
        let mut reg = Registry::load(&state, Path::new("shed.sock"), Vec::new()).unwrap();
        let cmd = ["sh", "-c", "trap '' HUP; sleep 60 & exit 3"];
        let run = json!({"name": "left", "cwd": "/", "cmd": cmd});
        reg.run(serde_json::from_value(run).unwrap()).unwrap();
        let mut runner = Runner::new(&state);
        let observe = |reg: &Registry, runner: &mut Runner, out: &mut Out| {
            let procs = proctree::system().list().unwrap();
            let infos = reg.ls(&LsParams::default());
            let seen =
                terminal::observe(&infos, &reg.leaders(), Some(&procs), &runner.terminal.seen);
            runner.terminal.observe(seen, out);
        };
        let mut o = out(0);
        observe(&reg, &mut runner, &mut o);
        let term = Arc::clone(reg.live("left").unwrap());
        let leader = term.pid();
        let deadline = shed_core::clock::now() + Duration::from_secs(10);
        term.wait_until(|_| false, Some(deadline)).await.unwrap();
        reg.refresh().unwrap();
        assert!(group_alive(leader), "the child outlives its program");
        let mut o = out(5);
        observe(&reg, &mut runner, &mut o);
        for now in [15, 25, 35] {
            o.now = now;
            runner.tick(&mut o);
        }
        proctree::kill_tree(&proctree::system(), leader, Duration::ZERO, || {});
        for _ in 0..100 {
            if !group_alive(leader) {
                break;
            }
            shed_core::clock::sleep(Duration::from_millis(50));
        }
        o.now = 40;
        observe(&reg, &mut runner, &mut o);
        runner.tick(&mut o);
        assert_eq!(o.lines.len(), 1, "{:?}", o.lines);
        let line = &o.lines[0];
        assert_eq!((line.kind, line.row.as_str()), (Differs::Action, "T9"));
        assert_eq!(
            (line.table.as_str(), line.code.as_str()),
            ("end-tree", "exited")
        );
        let state_now = runner.terminal.subjects["left"].tracker.state();
        assert_eq!(state_now, shed_proto::flow::Life::Gone);
        shadow::write(&state, &o.lines, Some(&runner.reports())).unwrap();
        let report = report(&state, None, 40);
        let terminal = flow(&report.flows, shed_proto::flow::FlowName::Terminal);
        assert_eq!(terminal.stuck.get("tree-left"), Some(&1));
        assert_eq!(terminal.fired.get("T26"), Some(&1));
        assert!(!terminal.never_seen.contains(&"empty".to_owned()));
        assert_eq!(terminal.disagreements[0].sample, "left");
    }

    fn key(state: KeyState) -> ProxyKey {
        ProxyKey {
            id: "3f9a1c2e".into(),
            terminal: "box/lane".into(),
            provider: "agent".into(),
            account: "work".into(),
            created: 0,
            state,
        }
    }

    #[test]
    fn a_key_today_s_code_ends_under_a_running_lane_is_one_state_line_in_the_report() {
        let state = crate::temp_dir("flows-key").join("state.json");
        let lane = json!({"name": "box/lane", "labels": {}, "cwd": "/", "cmd": ["agent"],
            "keep": false, "state": "running", "exit": null, "pid": 7});
        let there = BTreeMap::from([(
            "box".to_owned(),
            Some(vec![serde_json::from_value(lane).unwrap()]),
        )]);
        let mut runner = Runner::new(&state);
        let mut o = out(0);
        runner
            .key
            .observe(key::observe(&[key(KeyState::Live)], &there), &mut o);
        runner.tick(&mut o);
        o.now = 60;
        runner
            .key
            .observe(key::observe(&[key(KeyState::Ended)], &there), &mut o);
        for now in 60..=70 {
            o.now = now;
            runner.tick(&mut o);
        }
        assert_eq!(o.lines.len(), 1, "{:?}", o.lines);
        let line = &o.lines[0];
        assert_eq!((line.kind, line.at), (Differs::State, 63));
        assert_eq!(
            (line.table.as_str(), line.code.as_str()),
            ("in_use", "ended")
        );
        shadow::write(&state, &o.lines, Some(&runner.reports())).unwrap();
        let shown = report(&state, Some(61), 70);
        let keys = flow(&shown.flows, shed_proto::flow::FlowName::Key);
        assert_eq!((keys.mode, keys.subjects), (Mode::Shadow, 1));
        assert_eq!(keys.disagreements[0].count, 1);
        assert!(!keys.never_seen.contains(&"alive".to_owned()));
        assert!(keys.never_seen.contains(&"refused".to_owned()));
        assert!(
            report(&state, Some(64), 70)
                .flows
                .iter()
                .all(|f| f.disagreements.is_empty())
        );
    }

    #[test]
    fn a_link_that_reconnects_past_its_time_is_down_in_the_table_and_a_state_line() {
        let state = crate::temp_dir("flows-link").join("state.json");
        let mut runner = Runner::new(&state);
        let link = |state| {
            BTreeMap::from([(
                "box".to_owned(),
                shed_proto::Link {
                    state,
                    ..Default::default()
                },
            )])
        };
        let mut o = out(0);
        for (now, state) in [(0, LinkState::Up), (10, LinkState::Reconnecting)] {
            o.now = now;
            runner
                .link
                .observe(link::observe(&link(state), &runner.link.seen), &mut o);
        }
        for now in 10..=80 {
            o.now = now;
            runner.tick(&mut o);
        }
        assert_eq!(o.lines.len(), 1, "{:?}", o.lines);
        let line = &o.lines[0];
        assert_eq!(
            (line.kind, line.row.as_str(), line.at),
            (Differs::State, "L7", 73)
        );
        assert_eq!(
            (line.table.as_str(), line.code.as_str()),
            ("down", "reconnecting")
        );
        assert_eq!(runner.link.report.stuck.get("link-down"), Some(&1));
    }
}
