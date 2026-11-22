use std::path::PathBuf;

use shed_server::runner::ExitCode;

use crate::cx::{Cx, On};

pub const ON: On = On::No;

#[derive(clap::Args)]
pub struct Args {
    #[arg(long)]
    to: PathBuf,
}

pub fn run(cx: &Cx<'_>, Args { to }: Args) -> anyhow::Result<ExitCode> {
    let written = crate::skill::write(&to)?;
    cx.print(&written, || written.join("\n") + "\n")?;
    Ok(ExitCode::SUCCESS)
}
