use std::collections::BTreeSet;

use anyhow::{Context as _, bail};
use serde_json::{Value, json};
use shed_client::Calls as _;
use shed_server::proctree::{self, ProcessTable as _};
use shed_server::proto::{LsParams, TerminalInfo};
use shed_server::runner::ExitCode;

use crate::cx::{Cx, On};
use crate::table::{columns, table, totals};

pub const ON: On = On::Ssh;

#[derive(clap::Args)]
pub struct Args {
    name: Option<String>,
    #[arg(long, requires = "name")]
    full: bool,
}

pub fn run(cx: &Cx<'_>, args: Args) -> anyhow::Result<ExitCode> {
    let client = cx.server.connect(None)?;
    if let Some(name) = args.name {
        if cx.server.on.is_some() {
            bail!("shed top <terminal> reads this machine's process table; run it there");
        }
        processes(cx, &client.get(&name)?, args.full)?;
        return Ok(ExitCode::SUCCESS);
    }
    let mut list = client.ls(&LsParams::default())?;
    list.sort_by_key(|t| std::cmp::Reverse(t.resources.map(|r| r.rss_mb)));
    cx.print(&list, || table(&list, true) + &totals(&list))?;
    Ok(ExitCode::SUCCESS)
}

fn processes(cx: &Cx<'_>, info: &TerminalInfo, full: bool) -> anyhow::Result<()> {
    let pid = info
        .pid
        .with_context(|| format!("{} is not running", info.name))?;
    let procs = proctree::system().list().context("the process table")?;
    let mut tree = proctree::tree(&procs, pid, &BTreeSet::new());
    tree.retain(|p| p.rss_bytes > 50 << 20);
    tree.sort_by_key(|p| std::cmp::Reverse(p.rss_bytes));
    let rows: Vec<Value> = (tree.iter())
        .filter_map(|p| {
            let (cmd, age) = proctree::command(p.pid)?;
            let name = cmd.split_whitespace().next().unwrap_or_default();
            let name = name.rsplit('/').next().unwrap_or_default().to_owned();
            let cpu_pct = p.cpu.as_millis() * 100 / age.as_millis().max(1000);
            let cmd = if full { cmd } else { name };
            let (rss_mb, age_s) = (p.rss_bytes >> 20, age.as_secs());
            Some(json!({"pid": p.pid, "rss_mb": rss_mb, "cpu_pct": cpu_pct, "age_s": age_s, "cmd": cmd}))
        })
        .collect();
    cx.print(&rows, || {
        let head = ["PID", "MEM", "CPU", "AGE", "CMD"]
            .map(str::to_owned)
            .to_vec();
        let cell = |r: &Value, k: &str, unit: &str| format!("{}{unit}", r[k]);
        let body = rows.iter().map(|r| {
            let cmd = r["cmd"].as_str().unwrap_or_default().to_owned();
            let age = format!("{}m", r["age_s"].as_u64().unwrap_or(0) / 60);
            vec![
                cell(r, "pid", ""),
                cell(r, "rss_mb", "M"),
                cell(r, "cpu_pct", "%"),
                age,
                cmd,
            ]
        });
        columns(&[vec![head], body.collect()].concat())
    })
}
