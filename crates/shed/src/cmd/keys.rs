use serde_json::Value;
use shed_server::proto::{KeysParams, Method};
use shed_server::runner::ExitCode;

use crate::cx::{Cx, On};

pub const ON: On = On::Link;

#[derive(clap::Args)]
pub struct Args {
    name: String,
    #[arg(required = true)]
    keys: Vec<String>,
}

pub fn run(cx: &Cx<'_>, args: Args) -> anyhow::Result<ExitCode> {
    let params = KeysParams {
        name: cx.server.name(args.name),
        keys: args.keys,
    };
    let done: Value = cx.server.connect(None)?.call(Method::Keys, &params)?;
    cx.print(&done, String::new)?;
    Ok(ExitCode::SUCCESS)
}
