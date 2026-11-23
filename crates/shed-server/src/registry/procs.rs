use super::{Entry, Registry};
use shed_core::proctree::{self, ProcessInfo};
use shed_proto::{Proc, RpcError, State};
use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet};

fn row(p: &ProcessInfo, depth: u32) -> Proc {
    let cpu = p.cpu.as_millis() * 100 / p.elapsed.as_millis().max(1000);
    Proc {
        pid: p.pid,
        ppid: p.ppid,
        depth,
        cpu_pct: u64::try_from(cpu).unwrap_or(u64::MAX),
        rss_mb: p.rss_bytes >> 20,
        elapsed_s: p.elapsed.as_secs(),
        ..Proc::default()
    }
}

pub(crate) fn with_commands(mut rows: Vec<Proc>) -> Vec<Proc> {
    let pids: Vec<i32> = rows.iter().map(|r| r.pid).collect();
    let mut commands = proctree::commands(&pids);
    for row in &mut rows {
        row.command = commands.remove(&row.pid).unwrap_or_default();
    }
    rows
}

impl Entry {
    pub(super) fn own(&self, pid: i32) -> bool {
        let acp = self.info.acp.is_some();
        Some(pid) == self.info.pid || (acp && self.agent.as_ref().is_none_or(|a| a.contains(&pid)))
    }

    fn tree(&self, procs: &[ProcessInfo]) -> Vec<ProcessInfo> {
        match (self.info.state, self.info.pid) {
            (State::Running, Some(pid)) => proctree::tree(procs, pid, &BTreeSet::new()),
            _ => Vec::new(),
        }
    }
}

impl Registry {
    pub(crate) fn leaders(&self) -> BTreeMap<String, i32> {
        let leader = |(n, e): (&String, &Entry)| Some((n.clone(), e.term.as_ref()?.pid()));
        self.entries.iter().filter_map(leader).collect()
    }

    pub(crate) fn procs(&self, name: &str, procs: &[ProcessInfo]) -> Result<Vec<Proc>, RpcError> {
        let entry = self.entry(name)?;
        let tree = entry.tree(procs);
        let roots = tree
            .iter()
            .filter(|p| !tree.iter().any(|q| q.pid == p.ppid));
        let mut stack: Vec<(u32, &ProcessInfo)> = roots.rev().map(|p| (0, p)).collect();
        let mut rows = Vec::new();
        while let Some((depth, p)) = stack.pop() {
            rows.push(Proc {
                own: entry.own(p.pid),
                ..row(p, depth)
            });
            let children = tree.iter().filter(|c| c.ppid == p.pid).rev();
            stack.extend(children.map(|c| (depth + 1, c)));
        }
        Ok(rows)
    }

    pub(crate) fn top(&self, procs: &[ProcessInfo], n: usize) -> [Vec<Proc>; 2] {
        let mut held = BTreeMap::new();
        for (name, entry) in &self.entries {
            for p in entry.tree(procs) {
                held.entry(p.pid).or_insert_with(|| name.clone());
            }
        }
        let live = procs.iter().filter(|p| !p.zombie);
        let terminal = |p: &ProcessInfo| held.get(&p.pid).cloned();
        let mut rows: Vec<Proc> = (live.map(|p| Proc {
            terminal: terminal(p),
            ..row(p, 0)
        }))
        .collect();
        rows.sort_by_key(|r| Reverse(r.cpu_pct));
        let cpu = rows.iter().take(n).cloned().collect();
        rows.sort_by_key(|r| Reverse(r.rss_mb));
        rows.truncate(n);
        [cpu, rows]
    }
}
