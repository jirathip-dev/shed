use serde_json::{Value, json};
use shed_client::Calls as _;
use shed_server::proto::{LsParams, Proc};
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
        processes(cx, client.procs(&name)?, args.full)?;
        return Ok(ExitCode::SUCCESS);
    }
    let mut list = client.ls(&LsParams::default())?;
    list.sort_by_key(|t| std::cmp::Reverse(t.resources.map(|r| r.rss_mb)));
    cx.print(&list, || table(&list, true) + &totals(&list))?;
    Ok(ExitCode::SUCCESS)
}

fn processes(cx: &Cx<'_>, mut tree: Vec<Proc>, full: bool) -> anyhow::Result<()> {
    tree.retain(|p| p.rss_mb > 50);
    tree.sort_by_key(|p| std::cmp::Reverse(p.rss_mb));
    let rows: Vec<Value> = (tree.into_iter())
        .map(|p| {
            let name = p.command.split_whitespace().next().unwrap_or_default();
            let name = name.rsplit('/').next().unwrap_or_default().to_owned();
            let cmd = if full { p.command } else { name };
            json!({"pid": p.pid, "rss_mb": p.rss_mb, "cpu_pct": p.cpu_pct, "age_s": p.elapsed_s, "cmd": cmd})
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
