use shed_server::runner::ExitCode;

use crate::cx::{Cx, On};
use crate::service;

pub const ON: On = On::No;

#[derive(clap::Args)]
pub struct Args {
    #[arg(long)]
    force: bool,
}

pub fn run(_: &Cx<'_>, Args { force }: Args) -> anyhow::Result<ExitCode> {
    service::install(force)?;
    Ok(ExitCode::SUCCESS)
}
