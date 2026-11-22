use shed_client::Calls as _;
use shed_server::runner::ExitCode;

use crate::cx::{Cx, On};

pub const ON: On = On::Link;

#[derive(clap::Args)]
pub struct Args {
    name: String,
}

pub fn run(cx: &Cx<'_>, args: Args) -> anyhow::Result<ExitCode> {
    let info = cx.server.connect(None)?.get(&cx.server.name(args.name))?;
    let pretty = serde_json::to_string_pretty(&info)?;
    cx.print(&info, || format!("{pretty}\n"))?;
    Ok(ExitCode::SUCCESS)
}
