use shed_server::runner::ExitCode;

use crate::cx::{Cx, On};
use crate::service;

pub const ON: On = On::No;

#[derive(clap::Args)]
pub struct Args {}

pub fn run(_: &Cx<'_>, _: Args) -> anyhow::Result<ExitCode> {
    service::uninstall()?;
    Ok(ExitCode::SUCCESS)
}
