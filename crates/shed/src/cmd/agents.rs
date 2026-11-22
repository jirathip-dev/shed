use clap::Subcommand;
use shed_server::proto::SessionRecord;
use shed_server::runner::ExitCode;
use shed_server::{agents, lane};

use crate::cx::{Cx, On};

pub const ON: On = On::No;

#[derive(clap::Args)]
pub struct Args {
    #[command(subcommand)]
    action: Option<Action>,
}

#[derive(Subcommand)]
enum Action {
    Add {
        id: String,
        #[arg(long, value_name = "SRC", default_value = agents::REGISTRY)]
        registry: String,
    },
    Rm {
        id: String,
    },
    Sessions {
        id: String,
    },
}

pub fn run(cx: &Cx<'_>, args: Args) -> anyhow::Result<ExitCode> {
    let paths = cx.paths;
    let line = |a: &agents::Agent| format!("{} {} ({})\n", a.id, a.version, a.distribution);
    match args.action {
        None => {
            let list = shed_server::providers::known(&paths.state, &paths.providers())?;
            cx.print(&list, || {
                list.iter()
                    .filter_map(|a| a.installed.as_ref())
                    .map(|a| format!("{}  {}", line(a).trim_end(), a.cmd.join(" ")) + "\n")
                    .collect()
            })
        }
        Some(Action::Add { id, registry }) => {
            let agent = agents::add(&paths.state, &id, &registry)?;
            cx.print(&agent, || format!("installed {}", line(&agent)))
        }
        Some(Action::Rm { id }) => {
            let agent = agents::rm(&paths.state, &id)?;
            cx.print(&agent, || format!("removed {}", line(&agent)))
        }
        Some(Action::Sessions { id }) => {
            let mut list = lane::sessions::list(&paths.state)?;
            list.retain(|s| s.agent == id);
            cx.print(&list, || {
                let row = |s: &SessionRecord| {
                    let (id, updated, cwd) = (&s.id, s.updated, s.cwd.display());
                    format!("{id}  {updated}  {cwd}  {}\n", s.title)
                };
                list.iter().map(row).collect()
            })
        }
    }?;
    Ok(ExitCode::SUCCESS)
}
