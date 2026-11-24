//! The program's state: what it reported, the silence fallback, and waiting on them.

use std::time::{Duration, Instant};

use tokio::sync::watch;

use super::Terminal;
use crate::Error;
use crate::runner::ExitStatus;
use crate::screen::Report;

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
    /// The state the program last reported (`shed state`, the OSC 7501 escape or another agent's). Without
    /// one, `idle` once silent for the
    /// [`set_idle_after`](Self::set_idle_after) time; `stuck` while a [`write`](Self::write) waits.
    #[must_use]
    pub fn report(&self) -> Option<Report> {
        let status = self.watch.borrow();
        if status.stuck {
            return Some(Report {
                state: "stuck".to_owned(),
                note: Some("its input is not drained".to_owned()),
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
    /// output chunk and report and when the idle fallback comes due (no polling).
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
            let wake = [deadline, idle].into_iter().flatten().min();
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
