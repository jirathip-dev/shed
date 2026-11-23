use shed_server::runner::ExitCode;

use crate::cx::{Cx, On};

pub const ON: On = On::Ssh;

#[derive(clap::Args)]
pub struct Args {
    name: String,
}

pub fn run(cx: &Cx<'_>, Args { name }: Args) -> anyhow::Result<ExitCode> {
    crate::attach::attach(cx.server.connect(None)?, &name)?;
    Ok(ExitCode::SUCCESS)
}
