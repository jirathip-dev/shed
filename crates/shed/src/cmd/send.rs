use serde_json::json;
use shed_client::Calls as _;
use shed_server::proto::SendParams;
use shed_server::runner::ExitCode;

use crate::cx::{Cx, On};

pub const ON: On = On::Link;

#[derive(clap::Args)]
pub struct Args {
    name: String,
    text: String,
    #[arg(long)]
    no_enter: bool,
    #[arg(long)]
    no_paste: bool,
    #[arg(long, value_name = "SECONDS")]
    draft_wait: Option<u64>,
}

pub fn run(cx: &Cx<'_>, args: Args) -> anyhow::Result<ExitCode> {
    let params = SendParams {
        name: cx.server.name(args.name),
        text: args.text,
        no_enter: args.no_enter,
        no_paste: args.no_paste,
        draft_wait: args.draft_wait,
    };
    let client = cx.server.connect(None)?;
    if client.get(&params.name)?.acp.is_some() {
        client.prompt(&params.name, &params.text)?;
    } else {
        client.send(&params)?;
    }
    cx.print(&json!({}), String::new)?;
    Ok(ExitCode::SUCCESS)
}
