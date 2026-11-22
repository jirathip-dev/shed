//! The program's state: what it reported, the screen rules, the silence fallback, and waiting on them.

use std::time::{Duration, Instant};

use tokio::sync::watch;

use super::{Terminal, lock};
use crate::Error;
use crate::runner::ExitStatus;
use crate::screen::Report;

const RULE_EVERY: Duration = Duration::from_millis(250);

/// A screen rule: while one of `any` shows in the last `region` non-blank lines of the screen, the state is
/// `state`. Above priority 0 it outranks the reported state; at or below, it applies only without one.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Rule {
    /// Its name; the state's `from` is `rule:<id>`.
    pub id: String,
    /// The state it sets.
    pub state: String,
    /// Why it is blocked.
    pub kind: Option<String>,
    /// The highest matching rule wins; the reported state has priority 0.
    pub priority: i32,
    /// How many of the screen's last non-blank lines it looks at.
    pub region: usize,
    /// The texts it matches, any one of them.
    pub any: Vec<String>,
}

#[derive(Default)]
pub(super) struct Rules {
    list: Vec<Rule>,
    seen: Option<u64>,
    at: Option<Instant>,
    hit: Option<Rule>,
    checks: u64,
}

/// Why [`Terminal::wait_until`] returned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Wake {
    /// The condition holds.
    Matched,
    /// The program exited first.
    Exited,
    /// The deadline passed first.
    Timeout,
}

pub(super) struct Status {
    pub(super) changes: u64,
    pub(super) ended: bool,
    pub(super) last_output: Instant,
    pub(super) report: Option<Report>,
    pub(super) records: Vec<Report>,
    pub(super) idle_after: Option<Duration>,
    pub(super) stuck: bool,
    pub(super) exit: Option<Result<ExitStatus, std::io::ErrorKind>>,
}

impl Status {
    pub(super) fn apply(&mut self, report: Report) {
        match (report.state.as_str(), report.id.clone()) {
            ("clear", None) => (self.report, self.records) = (None, Vec::new()),
            ("clear", Some(id)) => self.records.retain(|r| {
                r.id.as_ref()
                    .is_some_and(|r| *r != id && !r.starts_with(&format!("{id}/")))
            }),
            (_, None) => self.report = Some(report),
            (_, Some(id)) => {
                self.records.retain(|r| r.id.as_ref() != Some(&id));
                self.records.push(report);
                if self.records.len() > 256 {
                    self.records.remove(0);
                }
            }
        }
        self.end(self.ended);
    }

    pub(super) fn end(&mut self, ended: bool) {
        self.ended = ended;
        let lasts = |r: &Report| !ended || !["working", "blocked"].contains(&r.state.as_str());
        self.report = self.report.take().filter(lasts);
        self.records.retain(lasts);
    }

    fn idle_at(&self) -> Option<Instant> {
        match self.report {
            Some(_) => None,
            None => self
                .idle_after
                .and_then(|d| self.last_output.checked_add(d)),
        }
    }
}

pub(super) fn bump(watch: &watch::Sender<Status>, change: impl FnOnce(&mut Status)) {
    watch.send_modify(|status| {
        change(status);
        status.changes += 1;
    });
}

impl Terminal {
    /// The state the program last reported (`shed state`, the OSC 7501 escape or another agent's), unless a
    /// [`set_rules`](Self::set_rules) rule outranks it. Without either, `idle` once silent for the
    /// [`set_idle_after`](Self::set_idle_after) time; `stuck` while a [`write`](Self::write) waits.
    #[must_use]
    pub fn report(&self) -> Option<Report> {
        let rule = self.screen_rule();
        let status = self.watch.borrow();
        if status.stuck {
            return Some(Report {
                state: "stuck".to_owned(),
                note: Some("its input is not drained".to_owned()),
                ..Report::default()
            });
        }
        let outranks = |r: &Rule| r.priority > 0 || status.report.is_none();
        if let Some(r) = rule.filter(|r| !status.ended && outranks(r)) {
            return Some(Report {
                state: r.state,
                kind: r.kind,
                from: Some(format!("rule:{}", r.id)),
                ..Report::default()
            });
        }
        let idle = status.idle_at().is_some_and(|at| crate::clock::now() >= at);
        let from = |from: &str| Some(from.to_owned());
        let reported = status.report.clone().map(|r| Report {
            from: from("reported"),
            ..r
        });
        reported.or_else(|| {
            idle.then(|| Report {
                state: "idle".to_owned(),
                from: from("silence"),
                ..Report::default()
            })
        })
    }

    /// Sets the screen rules [`report`](Self::report) checks, at most every 250 ms and only after the
    /// screen changed.
    pub fn set_rules(&self, rules: Vec<Rule>) {
        *lock(&self.rules) = Rules {
            list: rules,
            ..Rules::default()
        };
    }

    /// How many times the screen rules have been checked.
    #[must_use]
    pub fn rule_checks(&self) -> u64 {
        lock(&self.rules).checks
    }

    fn screen_rule(&self) -> Option<Rule> {
        let mut rules = lock(&self.rules);
        if rules.list.is_empty() {
            return None;
        }
        let (changes, now) = (self.changes(), crate::clock::now());
        let due = (rules.at).and_then(|at| at.checked_add(RULE_EVERY));
        if rules.seen != Some(changes) && due.is_none_or(|at| now >= at) {
            let text = self.screen();
            let lines: Vec<&str> = text.lines().collect();
            let end = lines.iter().rposition(|l| !l.trim().is_empty());
            let region = |n| lines.iter().take(end.map_or(0, |i| i + 1)).rev().take(n);
            let shows = |r: &&Rule| region(r.region).any(|l| r.any.iter().any(|s| l.contains(s)));
            let hit = rules.list.iter().filter(shows).max_by_key(|r| r.priority);
            rules.hit = hit.cloned();
            (rules.seen, rules.at, rules.checks) = (Some(changes), Some(now), rules.checks + 1);
        }
        rules.hit.clone()
    }

    fn rule_due(&self) -> Option<Instant> {
        let rules = lock(&self.rules);
        let stale = !rules.list.is_empty() && rules.seen != Some(self.changes());
        rules.at.filter(|_| stale)?.checked_add(RULE_EVERY)
    }

    /// The records the program reported with an `id`, least recently updated first.
    #[must_use]
    pub fn records(&self) -> Vec<Report> {
        self.watch.borrow().records.clone()
    }

    /// Records a reported state, as if the program had sent the escape.
    pub fn set_report(&self, report: Report) {
        bump(&self.watch, |s| s.apply(report));
    }

    /// Turns the silence fallback on (`Some`) or off: [`report`](Self::report) says `idle` after this
    /// long without output, until the program reports a state.
    pub fn set_idle_after(&self, after: Option<Duration>) {
        bump(&self.watch, |s| s.idle_after = after);
    }

    /// Waits until `done` holds, the program exits or `deadline` passes, checking `done` again on every
    /// output chunk and report and when the idle fallback or a screen rule check comes due (no polling).
    /// Needs a tokio runtime with time.
    ///
    /// # Errors
    /// [`Error::Io`] when the exit status cannot be read.
    pub async fn wait_until(
        &self,
        mut done: impl FnMut(&Self) -> bool,
        deadline: Option<Instant>,
    ) -> Result<Wake, Error> {
        let mut changed = self.watch.subscribe();
        loop {
            changed.mark_unchanged();
            if done(self) {
                return Ok(Wake::Matched);
            }
            if self.try_wait()?.is_some() {
                return Ok(Wake::Exited);
            }
            let now = crate::clock::now();
            if deadline.is_some_and(|d| now >= d) {
                return Ok(Wake::Timeout);
            }
            let idle = changed.borrow().idle_at().filter(|at| *at > now);
            let due = self.rule_due().filter(|at| *at > now);
            let wake = [deadline, idle, due].into_iter().flatten().min();
            let sleep = async {
                match wake {
                    Some(at) => tokio::time::sleep_until(at.into()).await,
                    None => std::future::pending().await,
                }
            };
            tokio::select! {
                _ = changed.changed() => {}
                () = sleep => {}
            }
        }
    }
}
