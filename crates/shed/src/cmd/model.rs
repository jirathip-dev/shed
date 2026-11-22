use serde_json::json;
use shed_client::Calls as _;
use shed_server::runner::ExitCode;

use crate::cx::{Cx, On};
use crate::out;

pub const ON: On = On::Link;

#[derive(clap::Args)]
pub struct Args {
    name: String,
    model: String,
}

pub fn run(cx: &Cx<'_>, args: Args) -> anyhow::Result<ExitCode> {
    let name = cx.server.name(args.name);
    let client = cx.server.connect(None)?;
    let info = client.get(&name)?;
    client.model(&name, &args.model)?;
    if info.session.is_some_and(|s| s.turn_ended.is_some()) {
        out::err(
            "shed: the model switch drops the prompt cache; the next turn reads the conversation uncached",
        );
    }
    cx.print(&json!({}), String::new)?;
    Ok(ExitCode::SUCCESS)
}
