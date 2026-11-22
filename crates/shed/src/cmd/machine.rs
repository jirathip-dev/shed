use std::fmt::Write as _;
use std::path::Path;

use clap::Subcommand;
use serde::Serialize;
use shed_client::Calls as _;
use shed_server::machines::{self, Machine};
use shed_server::proto::{Link, MachineStats, StatsParams};
use shed_server::runner::ExitCode;

use crate::cx::{Cx, On, links};

#[derive(clap::Args)]
pub struct Args {
    #[command(subcommand)]
    command: Action,
}

#[derive(Subcommand)]
enum Action {
    Add {
        name: String,
        target: String,
    },
    Ls,
    Rm {
        name: String,
    },
    Stats {
        #[arg(long, value_name = "T")]
        since: Option<u64>,
    },
}

pub fn on(args: &Args) -> On {
    match args.command {
        Action::Stats { .. } => On::Ssh,
        _ => On::No,
    }
}

pub fn run(cx: &Cx<'_>, args: Args) -> anyhow::Result<ExitCode> {
    let paths = cx.paths;
    match args.command {
        Action::Add { name, target } => {
            let m = machines::add(&paths.config, &name, &target)?;
            links(paths);
            cx.print(&m, || format!("{} added\n", m.name))
        }
        Action::Ls => {
            let links = links(paths);
            let list: Vec<Listed> = machines::load(&paths.config)?
                .into_iter()
                .map(|machine| Listed {
                    link: links.get(&machine.name).cloned(),
                    machine,
                })
                .collect();
            cx.print(&list, || {
                let width = list.iter().map(|l| l.machine.name.len()).max();
                let width = width.unwrap_or(0).max(4);
                let mut out = format!("{:<width$}  TARGET\n", "NAME");
                for Listed { machine: m, link } in &list {
                    let link = link.as_ref().map_or_else(String::new, |l| {
                        format!("  {:?} since {}", l.state, l.since).to_lowercase()
                    });
                    writeln!(out, "{:<width$}  {}{link}", m.name, m.target).ok();
                }
                out
            })
        }
        Action::Rm { name } => {
            let m = machines::remove(&paths.config, &name)?;
            cx.print(&m, || format!("{} removed\n", m.name))
        }
        Action::Stats { since } => {
            return stats(cx, StatsParams {
                    since,
                    ..StatsParams::default()
                });
        }
    }?;
    Ok(ExitCode::SUCCESS)
}

#[derive(Serialize)]
struct Listed {
    #[serde(flatten)]
    machine: Machine,
    #[serde(skip_serializing_if = "Option::is_none")]
    link: Option<Link>,
}

fn stats(cx: &Cx<'_>, params: StatsParams) -> anyhow::Result<ExitCode> {
    let server = &cx.server;
    let stats: MachineStats = match &server.on {
        None => server.connect(None)?.stats(&params)?,
        Some(m) => {
            let control = server.paths.socket.parent().unwrap_or(Path::new("."));
            let seen = server.paths.state.with_file_name("seen");
            match machines::stats(m, control, &seen, params)? {
                Ok(stats) => stats,
                Err(offline) => {
                    cx.print(&offline, || match offline.last_seen {
                        Some(t) => format!("{} offline, last seen {t}\n", m.name),
                        None => format!("{} offline, never seen\n", m.name),
                    })?;
                    return Ok(ExitCode::FAILURE);
                }
            }
        }
    };
    let s = &stats.now;
    let terminals: Vec<String> = s
        .terminals
        .iter()
        .map(|(k, n)| format!("{k}={n}"))
        .collect();
    cx.print(&stats, || {
        let (load, mem, swap, disk) = (
            s.load_per_cpu,
            s.mem_pressure_pct,
            s.swap_used_mb,
            s.disk_free_mb,
        );
        let terminals = terminals.join(",");
        format!(
            "load/cpu {load:.2}  memory {mem}%  swap {swap}M  disk free {disk}M  cpus {}  up {}s  terminals {terminals}\n",
            s.cpus, s.uptime_s
        )
    })?;
    Ok(ExitCode::SUCCESS)
}
