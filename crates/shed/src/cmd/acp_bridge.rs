use std::path::PathBuf;

use anyhow::Context as _;
use shed_server::lane::{self, Policy};
use shed_server::runner::ExitCode;

use crate::args::policy;
use crate::attach::RawMode;
use crate::cx::{Cx, On};

pub const ON: On = On::No;

#[derive(clap::Args)]
#[command(hide = true)]
pub struct Args {
    #[arg(long)]
    socket: PathBuf,
    #[arg(long)]
    state: PathBuf,
    #[arg(long, value_parser = policy)]
    policy: Policy,
    #[arg(long)]
    agent: String,
    #[arg(long)]
    providers: PathBuf,
    #[arg(long)]
    stream: bool,
    #[arg(long)]
    why: Option<String>,
    #[arg(last = true, required = true)]
    cmd: Vec<String>,
}

pub fn run(_: &Cx<'_>, Args { socket, state, policy, agent, providers, stream, why, cmd }: Args) -> anyhow::Result<ExitCode> {
    let here = std::env::current_dir().context("current directory")?;
    let _raw = RawMode::enter(1, 0)?;
    let catalog = shed_server::providers::Catalog::load(&providers).0;
    let rules = catalog.by_id(&agent).cloned().unwrap_or_default();
    let at = (socket.as_path(), state.as_path());
    let driver = (stream, why.as_deref());
    let code = lane::bridge(&cmd, &here, at, (&agent, policy, rules), driver)?;
    Ok(ExitCode::from(u8::try_from(code).unwrap_or(1)))
}
