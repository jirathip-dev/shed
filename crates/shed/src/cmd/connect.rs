use std::os::unix::net::UnixStream;

use anyhow::{Context as _, bail};
use shed_server::runner::ExitCode;

use crate::cx::{Cx, On, connect};

pub const ON: On = On::No;

#[derive(clap::Args)]
pub struct Args {
    #[arg(long)]
    autostart: bool,
}

pub fn run(cx: &Cx<'_>, Args { autostart }: Args) -> anyhow::Result<ExitCode> {
    let paths = cx.paths;
    if UnixStream::connect(&paths.socket).is_err() {
        if !autostart {
            bail!(
                "no shed server on {}; start one with `shed serve --detach`",
                paths.socket.display()
            );
        }
        connect(paths, cx.dir, Some(true), None)?;
    }
    shed_server::relay(&paths.socket).context("shed connect")?;
    Ok(ExitCode::SUCCESS)
}
