use anyhow::Context as _;
use shed_server::install;
use shed_server::runner::ExitCode;

use crate::cx::{Cx, On};
use crate::service;

pub const ON: On = On::No;

#[derive(clap::Args)]
#[command(args_conflicts_with_subcommands = true)]
pub struct Args {
    #[arg(long)]
    force: bool,
    #[command(subcommand)]
    action: Option<Action>,
}

#[derive(clap::Subcommand)]
enum Action {
    Ls,
}

pub fn run(cx: &Cx<'_>, Args { force, action }: Args) -> anyhow::Result<ExitCode> {
    if let Some(Action::Ls) = action {
        let exe = std::env::current_exe().context("this binary's path")?;
        let i = install::ls(&exe);
        let row = |name: &str, v: &Option<String>| format!("{name:<7} {}\n", v.as_deref().unwrap_or("-"));
        let text = || [("live", &i.live), ("staged", &i.staged), ("prev", &i.prev)].map(|(n, v)| row(n, v)).concat();
        cx.print(&i, text)?;
    } else {
        service::install(force)?;
    }
    Ok(ExitCode::SUCCESS)
}
