use std::io;

use anyhow::Context as _;
use shed_client::Calls as _;
use shed_server::config;
use shed_server::proto::Reported;
use shed_server::runner::ExitCode;

use crate::cx::{Cx, On};

pub const ON: On = On::Link;

#[derive(clap::Args)]
pub struct Args {
    #[arg(required = true, num_args = 1..=2, value_name = "[T] STATE")]
    args: Vec<String>,
    #[arg(long)]
    note: Option<String>,
}

pub fn run(cx: &Cx<'_>, args: Args) -> anyhow::Result<ExitCode> {
    let Args { mut args, note } = args;
    let state = args.pop().unwrap_or_default();
    let name = match args.pop() {
        Some(name) => name,
        None => config::var("SHED")
            .context("no terminal given and $SHED is unset (it is set inside a terminal)")?,
    };
    let report = Reported {
        state,
        note,
        ..Reported::default()
    };
    let server = &cx.server;
    let info = server
        .connect_within(None, Some(config::state_timeout()))
        .and_then(|c| Ok(c.state(&server.name(name), &report)?))
        .map_err(|e| match e.downcast_ref() {
            Some(shed_client::Error::Io { source, .. })
                if source.kind() == io::ErrorKind::TimedOut =>
            {
                anyhow::anyhow!("{source} ($SHED_STATE_TIMEOUT)")
            }
            _ => e,
        })?;
    cx.print(&info, String::new)?;
    Ok(ExitCode::SUCCESS)
}
