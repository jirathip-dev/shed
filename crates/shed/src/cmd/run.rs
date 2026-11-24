use std::path::PathBuf;

use anyhow::{Context as _, bail};
use serde_json::{Map, Value};
use shed_client::Calls as _;
use shed_server::lane::Policy;
use shed_server::proto::{Parent, RunParams};
use shed_server::runner::ExitCode;

use crate::args::{label, mem, policy, size};
use crate::cx::{Cx, On};

pub const ON: On = On::Link;

#[derive(clap::Args)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "each is an independent command-line switch"
)]
pub struct Args {
    #[arg(long)]
    name: Option<String>,
    #[arg(long = "label", value_name = "K=V", value_parser = label)]
    labels: Vec<(String, String)>,
    #[arg(long)]
    cwd: Option<PathBuf>,
    #[arg(long)]
    keep: bool,
    #[arg(long)]
    autostart: bool,
    #[arg(long, value_name = "S")]
    idle_after: Option<u64>,
    #[arg(long, value_name = "ID", conflicts_with_all = ["keep", "idle_after"])]
    acp: Option<String>,
    #[arg(long, requires = "acp", value_parser = policy, default_value = "ask")]
    policy: Policy,
    #[arg(long, value_name = "CMD", requires = "acp")]
    judge: Option<String>,
    #[arg(long, value_name = "S", requires = "judge")]
    judge_secs: Option<u64>,
    #[arg(long, value_name = "ID", requires = "acp")]
    resume: Option<String>,
    #[arg(long, value_name = "FILE", requires = "acp")]
    mcp: Option<PathBuf>,
    #[arg(long, value_name = "JSON", requires = "acp", value_parser = |s: &str| serde_json::from_str::<Map<String, Value>>(s))]
    meta: Option<Map<String, Value>>,
    #[arg(long, requires = "acp")]
    no_hibernate: bool,
    #[arg(long, requires = "acp", conflicts_with = "resume")]
    warm: bool,
    #[arg(long, requires = "acp")]
    model: Option<String>,
    #[arg(long, requires = "acp")]
    effort: Option<String>,
    #[arg(long, value_name = "NAME", requires = "acp")]
    account: Option<String>,
    #[arg(long = "env", value_name = "K=V", value_parser = label)]
    env: Vec<(String, String)>,
    #[command(flatten)]
    parent: ParentArgs,
    #[arg(long, value_name = "RxC", value_parser = size)]
    size: Option<(u16, u16)>,
    #[arg(long, value_name = "SIZE", value_parser = mem)]
    mem_max: Option<u64>,
    #[arg(last = true, required_unless_present = "acp")]
    cmd: Vec<String>,
}

#[derive(clap::Args)]
struct ParentArgs {
    #[arg(long, value_name = "T")]
    parent: Option<String>,
    #[arg(long, conflicts_with = "parent")]
    no_parent: bool,
    #[arg(long)]
    outlive_parent: bool,
    #[arg(long)]
    notify: bool,
}

pub fn run(cx: &Cx<'_>, args: Args) -> anyhow::Result<ExitCode> {
    let server = &cx.server;
    let cwd = match (&server.on, args.cwd) {
        (None, cwd) => {
            let here = std::env::current_dir().context("current directory")?;
            cwd.map_or_else(|| here.clone(), |d| here.join(d))
        }
        (Some(_), Some(d)) if d.is_absolute() => d,
        (Some(m), _) => bail!("with --on, pass --cwd as an absolute path on {}", m.name),
    };
    let mcp_servers = match &args.mcp {
        Some(file) => serde_json::from_slice(&std::fs::read(file)?)
            .with_context(|| format!("{}: not a JSON list", file.display()))?,
        None => Vec::new(),
    };
    let params = RunParams {
        name: args.name,
        labels: args.labels.into_iter().collect(),
        cwd,
        keep: args.keep,
        idle_after: args.idle_after,
        cmd: args.cmd,
        model: args.model,
        effort: args.effort,
        env: args.env.into_iter().collect(),
        policy: args.acp.is_some().then_some(args.policy),
        acp: args.acp,
        judge: (args.judge.iter())
            .flat_map(|j| j.split_whitespace().map(str::to_owned))
            .collect(),
        judge_secs: args.judge_secs,
        resume: args.resume,
        mcp_servers,
        meta: args.meta,
        no_hibernate: args.no_hibernate,
        warm: args.warm,
        parent: match (args.parent.no_parent, &server.on) {
            (true, _) => None,
            (false, None) => (args.parent.parent).or_else(|| shed_server::config::var("SHED")),
            (false, Some(_)) => args.parent.parent,
        }
        .map(|name| Parent {
            name,
            outlive: args.parent.outlive_parent,
            notify: args.parent.notify,
        }),
        machine: server.machine(),
        mem_max_mb: args.mem_max,
        rows: args.size.map(|s| s.0),
        cols: args.size.map(|s| s.1),
        account: args.account,
    };
    let info = server.connect(Some(args.autostart))?.run(&params)?;
    cx.print(&info, || format!("{}\n", info.name))?;
    Ok(ExitCode::SUCCESS)
}
