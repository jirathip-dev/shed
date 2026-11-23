use super::{Entry, Registry, family::note};
use crate::lane::sessions;
use shed_core::proctree::{self, ProcessInfo};
use shed_proto::{EventKind, MachineStats, Resources, Sample, State};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

#[derive(Default)]
pub(super) struct Guard {
    over: u8,
    last: Option<i32>,
    kills: u64,
    largest: Option<(i32, u64, String)>,
}

fn process_name(pid: i32) -> String {
    let cmd = proctree::command(pid).map(|c| c.0).unwrap_or_default();
    let first = cmd.split_whitespace().next().unwrap_or("?");
    first.rsplit('/').next().unwrap_or(first).to_owned()
}

fn largest<'a>(ps: &[&'a ProcessInfo]) -> Option<&'a ProcessInfo> {
    ps.iter().copied().max_by_key(|p| p.rss_bytes)
}

fn moved(old: u64, new: u64) -> bool {
    new.abs_diff(old) * 10 > old
}

impl Entry {
    fn guard(
        &mut self,
        procs: &[ProcessInfo],
        tree: &[ProcessInfo],
        limit_mb: u64,
    ) -> Option<(String, EventKind, Option<i32>)> {
        let pid = self.info.pid;
        let (mine, started): (Vec<_>, Vec<_>) = tree.iter().partition(|p| self.own(p.pid));
        let g = &mut self.guard;
        let (process, rss, by, end) = if crate::limit::scoped() {
            for p in &started {
                crate::limit::kill_first(p.pid);
            }
            let kills = pid.and_then(crate::limit::oom_kills).unwrap_or(g.kills);
            let now = largest(&started).map(|p| (p.pid, p.rss_bytes, process_name(p.pid)));
            let seen = std::mem::replace(&mut g.largest, now);
            if kills <= std::mem::replace(&mut g.kills, kills) {
                return None;
            }
            let alive = |pid: i32| tree.iter().any(|p| p.pid == pid);
            let gone = seen
                .filter(|s| !alive(s.0))
                .map(|(_, rss, name)| (rss, name));
            let dead = |p: &&ProcessInfo| p.zombie && alive(p.ppid);
            let zombie = || Some((0, crate::limit::comm(procs.iter().find(dead)?.pid)?));
            let (rss, name) = gone.or_else(zombie).unwrap_or((0, "?".to_owned()));
            (name, rss, "kernel", None)
        } else {
            let total: u64 = tree.iter().map(|p| p.rss_bytes).sum();
            let over = total > limit_mb.saturating_mul(1 << 20);
            g.over = if over { g.over.saturating_add(1) } else { 0 };
            if g.over < 2 {
                return None;
            }
            g.over = 0;
            let worker = largest(&started).map(|p| (p, "shed"));
            let (p, by) = worker.or_else(|| Some((largest(&mine)?, "none")))?;
            if g.last.replace(p.pid) == Some(p.pid) {
                return None;
            }
            let end = (by == "shed").then_some(p.pid);
            (process_name(p.pid), p.rss_bytes, by, end)
        };
        let rss_mb = rss >> 20;
        let line = format!("mem-limit {limit_mb}M: {process} {rss_mb}M, ended by {by}");
        let ended_by = by.to_owned();
        let kind = EventKind::MemLimit {
            limit_mb,
            process,
            rss_mb,
            ended_by,
        };
        Some((line, kind, end))
    }
}

impl Registry {
    pub(crate) fn sample(&mut self, procs: &[ProcessInfo], now: Instant) -> Vec<i32> {
        let (mut ended, mut notes) = (Vec::new(), Vec::new());
        for entry in self.entries.values_mut() {
            let Some(pid) = entry.info.pid else { continue };
            let tree = proctree::tree(procs, pid, &std::collections::BTreeSet::new());
            if tree.is_empty() {
                continue;
            }
            let usage = proctree::usage(&tree);
            let own = |p: &&ProcessInfo| {
                Some(p.pid) == entry.info.pid
                    || entry.agent.as_ref().is_none_or(|a| a.contains(&p.pid))
            };
            let agent: u64 = tree.iter().filter(own).map(|p| p.rss_bytes).sum();
            let acp = entry.info.acp.is_some();
            let (then, cpu) = entry.sampled;
            let wall = now.saturating_duration_since(then).as_micros().max(1);
            let busy = usage.cpu.saturating_sub(cpu).as_micros();
            entry.sampled = (now, usage.cpu);
            let cpu_pct = u64::try_from((busy * 100 + wall / 2) / wall).unwrap_or(u64::MAX);
            let now = Resources {
                rss_mb: usage.rss_bytes / (1 << 20),
                cpu_pct,
                agent_mb: acp.then_some(agent / (1 << 20)),
                work_mb: acp.then_some(usage.rss_bytes.saturating_sub(agent) / (1 << 20)),
            };
            entry.resources = Some(now);
            if entry
                .emitted
                .is_none_or(|old| moved(old.rss_mb, now.rss_mb) || moved(old.cpu_pct, now.cpu_pct))
            {
                entry.emitted = Some(now);
                self.events.emit(&entry.info, EventKind::Resources(now));
            }
            let limit = entry.info.mem_max_mb;
            if let Some((line, kind, end)) = limit.and_then(|mb| entry.guard(procs, &tree, mb)) {
                notes.extend(note(&entry.info, &line));
                self.events.emit(&entry.info, kind);
                ended.extend(end);
            }
        }
        if !notes.is_empty() {
            self.deliver(notes);
        }
        ended
    }

    pub(crate) fn dir(&self) -> PathBuf {
        self.path.parent().unwrap_or(Path::new(".")).to_owned()
    }

    fn counts(&self) -> BTreeMap<String, u64> {
        let mut counts = BTreeMap::new();
        for e in self.entries.values() {
            let state = match (e.info.state, &e.info.reported) {
                (State::Exited, _) => "exited",
                (State::Running, _) if e.recipe.is_some() => "spare",
                (State::Running, Some(r)) => &r.state,
                (State::Running, None) => "running",
            };
            *counts.entry(state.to_owned()).or_default() += 1;
        }
        counts
    }

    pub(crate) fn record(&mut self, mut sample: Sample) -> impl FnOnce() + Send + use<> {
        self.refresh().ok();
        let now = shed_core::clock::unix_now();
        self.prune(now);
        sample.terminals = self.counts();
        self.stats.push(sample);
        let saving = self.stats.saving();
        let running = self
            .entries
            .values()
            .filter(|e| e.info.state == State::Running);
        let live = running.filter_map(|e| Some(e.info.session.as_ref()?.id.clone()));
        let (state, live) = (self.path.clone(), live.collect());
        move || {
            saving();
            sessions::prune(&state, now, crate::config::sessions_keep(), &live);
        }
    }

    pub(crate) fn machine_stats(&self, mut now: Sample, since: Option<u64>) -> MachineStats {
        now.terminals = self.counts();
        let history = since.map(|t| self.stats.since(t)).unwrap_or_default();
        MachineStats {
            now,
            history,
            draining: self.drain.draining(),
            ..MachineStats::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::registry::tests::{dir, sleeper};
    use crate::registry::*;

    use shed_core::proctree::{self, ProcessInfo, ProcessTable as _};

    use shed_proto::{EventKind, EventsParams};
    use std::collections::BTreeMap;

    use std::io::{self, Write};
    use std::path::Path;
    use std::sync::Arc;
    use std::time::Duration;

    #[test]
    fn the_sampler_ends_a_child_over_the_limit_for_two_samples_and_leaves_the_rest() {
        if crate::limit::scoped() {
            writeln!(io::stderr(), "skipped: this host limits terminals by scope").ok();
            return;
        }
        let path = dir("mem-guard").join("state.json");
        let mut registry = Registry::load(&path, Path::new("shed.sock"), Vec::new()).unwrap();
        let run = |name: &str, limit: Option<u64>| {
            let run = serde_json::json!({"name": name, "cwd": "/", "cmd": ["sleep", "100"],
                "mem_max_mb": limit});
            serde_json::from_value(run).unwrap()
        };
        let mut terms = Vec::new();
        for (name, limit) in [("free", None), ("capped", Some(100))] {
            registry.run(run(name, limit)).unwrap();
            terms.push(Arc::clone(registry.live(name).unwrap()));
        }
        let pid = |n: usize| terms[n].pid();
        let proc = |id: i32, parent: i32, mb: u64| ProcessInfo {
            pid: id,
            ppid: parent,
            pgid: parent,
            rss_bytes: mb << 20,
            cpu: Duration::ZERO,
            elapsed: Duration::ZERO,
            zombie: false,
        };
        let (child, free_child) = (1_999_999_001, 1_999_999_002);
        let at = |agent_mb: u64, child_mb: u64| {
            let mut procs = vec![
                proc(pid(0), 1, 1),
                proc(free_child, pid(0), 5000),
                proc(pid(1), 1, agent_mb),
                proc(child, pid(1), child_mb),
            ];
            procs.retain(|p| p.rss_bytes > 0);
            procs
        };
        let mut sub = registry.events.subscribe(EventsParams {
            labels: BTreeMap::new(),
            since: Some(0),
        });
        let mut sample = |procs: Vec<ProcessInfo>| registry.sample(&procs, shed_core::clock::now());
        assert_eq!(sample(at(10, 80)), Vec::<i32>::new(), "under the limit");
        assert_eq!(sample(at(10, 80)), Vec::<i32>::new(), "under the limit");
        assert_eq!(sample(at(10, 150)), Vec::<i32>::new(), "one sample over");
        assert_eq!(sample(at(10, 80)), Vec::<i32>::new(), "back under");
        assert_eq!(sample(at(10, 150)), Vec::<i32>::new(), "one sample over");
        assert_eq!(sample(at(10, 150)), [child], "two in a row");
        assert_eq!(sample(at(10, 150)), Vec::<i32>::new(), "once per process");
        assert_eq!(sample(at(10, 150)), Vec::<i32>::new(), "once per process");
        assert_eq!(sample(at(150, 0)), Vec::<i32>::new(), "the agent itself");
        assert_eq!(
            sample(at(150, 0)),
            Vec::<i32>::new(),
            "the agent is never ended"
        );
        let limits: Vec<_> = (sub.events().into_iter())
            .filter_map(|e| match e.kind {
                EventKind::MemLimit {
                    rss_mb, ended_by, ..
                } => Some((e.name, rss_mb, ended_by)),
                _ => None,
            })
            .collect();
        let capped = |mb: u64, by: &str| ("capped".to_owned(), mb, by.to_owned());
        assert_eq!(limits, [capped(150, "shed"), capped(150, "none")]);
        for term in terms {
            term.kill(GRACE).unwrap();
        }
    }

    #[test]
    fn sample_time_report() {
        let path = dir("sample-time").join("state.json");
        let mut registry = Registry::load(&path, Path::new("shed.sock"), Vec::new()).unwrap();
        let terms: Vec<_> = (0..30)
            .map(|n| sleeper(&mut registry, &format!("s{n}")))
            .collect();
        let procs = proctree::system().list().unwrap();
        let started = shed_core::clock::now();
        for _ in 0..200 {
            registry.sample(&procs, shed_core::clock::now());
        }
        let each = started.elapsed() / 200;
        writeln!(io::stderr(), "sample of 30 terminals: {each:?}").ok();
        for term in terms {
            term.kill(GRACE).unwrap();
        }
    }
}
