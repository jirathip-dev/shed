use std::path::Path;

use anyhow::{Context as _, bail};
use shed_server::runner::ExitCode;

use crate::cx::{Cx, On};
use crate::out;

pub const ON: On = On::No;

#[derive(clap::Args)]
pub struct Args {
    agents: Vec<String>,
    #[arg(long)]
    remove: bool,
    #[arg(long)]
    dry_run: bool,
}

pub fn run(cx: &Cx<'_>, Args { agents, remove, dry_run }: Args) -> anyhow::Result<ExitCode> {
    let home = shed_server::config::var("HOME").context("HOME is not set")?;
    let (home, dir) = (Path::new(&home), cx.paths.providers());
    let files = shed_server::hooks::setup(home, &dir, &agents, remove, dry_run)?;
    if files.is_empty() {
        bail!("no installed agent has hooks shed can set up");
    }
    for f in files {
        let (agent, file) = (f.agent, f.file.display());
        match (f.after, dry_run) {
            (None, _) => out::line(format_args!("{agent}: {file} unchanged")),
            (Some(t), true) => out::text(format_args!("{agent}: {file} would become:\n{t}")),
            (Some(_), false) => out::line(format_args!("{agent}: {file} updated")),
        }
    }
    Ok(ExitCode::SUCCESS)
}
