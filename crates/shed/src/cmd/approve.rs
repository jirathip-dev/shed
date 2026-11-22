use shed_client::Calls as _;
use shed_server::runner::ExitCode;

use crate::cx::{Cx, On};

pub const ON: On = On::Link;

#[derive(clap::Args)]
pub struct Args {
    name: String,
    option: String,
    #[arg(long)]
    request: Option<String>,
}

pub fn run(cx: &Cx<'_>, args: Args) -> anyhow::Result<ExitCode> {
    let name = cx.server.name(args.name);
    let (option, request) = (args.option, args.request);
    cx.to_acp(&name, |c| c.approve(&name, &option, request.as_deref()))?;
    Ok(ExitCode::SUCCESS)
}
