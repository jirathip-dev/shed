use std::fmt::Write as _;

use shed_server::proto::{State, TerminalInfo};

pub fn totals(list: &[TerminalInfo]) -> String {
    let (mut awake, mut asleep, mut agent, mut work) = (0, 0, 0, 0);
    for t in list.iter().filter(|t| t.state == State::Running) {
        if state_word(t) == "hibernated" {
            asleep += 1;
        } else {
            awake += 1;
        }
        if let Some(r) = t.resources {
            agent += r.agent_mb.unwrap_or(0);
            work += r.work_mb.unwrap_or(r.rss_mb);
        }
    }
    format!("{awake} awake, {asleep} hibernated; agent {agent}M, work {work}M\n")
}

pub fn state_word(t: &TerminalInfo) -> &str {
    match (t.state, &t.reported) {
        (State::Running, Some(r)) if t.acp.is_some() => &r.state,
        (State::Running, _) => "running",
        (State::Exited, _) => "exited",
    }
}

pub fn table(list: &[TerminalInfo], top: bool) -> String {
    let dash = |v: Option<String>| v.unwrap_or_else(|| "-".to_owned());
    let mb = |v: Option<u64>| dash(v.map(|v| format!("{v}M")));
    let head = if top {
        "NAME STATE EXIT PID AGENT WORK CPU TOKENS CACHE CACHE-LIKELY LIMIT LABELS CMD"
    } else {
        "NAME STATE EXIT PID RSS CPU LABELS CMD"
    };
    let mut rows = vec![head.split(' ').map(str::to_owned).collect::<Vec<_>>()];
    for t in list {
        let labels: Vec<String> = t.labels.iter().map(|(k, v)| format!("{k}={v}")).collect();
        let usage = t.session.as_ref().and_then(|s| s.usage.as_ref());
        let r = t.resources;
        let mut row = vec![
            t.name.clone(),
            state_word(t).to_owned(),
            dash(t.exit.map(|v| v.to_string())),
            dash(t.pid.map(|v| v.to_string())),
        ];
        if top {
            row.push(mb(r.and_then(|r| r.agent_mb)));
            row.push(mb(r.map(|r| r.work_mb.unwrap_or(r.rss_mb))));
        } else {
            row.push(mb(r.map(|r| r.rss_mb)));
        }
        row.push(dash(r.map(|r| format!("{}%", r.cpu_pct))));
        if top {
            row.push(dash(
                usage.map(|u| format!("{}k/{}k", u.used / 1000, u.size / 1000)),
            ));
            let cache = usage.and_then(|u| u.turn?.cache_share());
            row.push(dash(cache.map(|p| format!("{p}%"))));
            row.push(dash(
                t.session.as_ref().and_then(|s| s.cache_likely.clone()),
            ));
            row.push(mb(t.mem_max_mb));
        }
        row.push(if labels.is_empty() {
            "-".to_owned()
        } else {
            labels.join(",")
        });
        row.push(t.cmd.join(" "));
        rows.push(row);
    }
    columns(&rows)
}

pub fn columns(rows: &[Vec<String>]) -> String {
    let mut widths = vec![0; rows.first().map_or(0, Vec::len)];
    for row in rows {
        for (w, cell) in widths.iter_mut().zip(row) {
            *w = cell.chars().count().max(*w);
        }
    }
    let mut out = String::new();
    for row in rows {
        let cells: Vec<String> = row
            .iter()
            .zip(&widths)
            .map(|(c, w)| format!("{c:<w$}"))
            .collect();
        writeln!(out, "{}", cells.join("  ").trim_end()).ok();
    }
    out
}
