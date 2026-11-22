use shed_client::Calls as _;
use shed_server::proto::LsParams;
use shed_server::runner::ExitCode;

use crate::args::label;
use crate::cx::{Cx, On};
use crate::table::table;

pub const ON: On = On::Link;

#[derive(clap::Args)]
pub struct Args {
    #[arg(long = "label", value_name = "K=V", value_parser = label)]
    labels: Vec<(String, String)>,
    #[arg(long, value_name = "T")]
    parent: Option<String>,
}

pub fn run(cx: &Cx<'_>, args: Args) -> anyhow::Result<ExitCode> {
    let labels = args.labels.into_iter().collect();
    let params = LsParams {
        labels,
        parent: args.parent,
    };
    let server = &cx.server;
    let mut list = server.connect(None)?.ls(&params)?;
    if server.linked {
        let prefix = server.name(String::new());
        list.retain(|t| t.name.starts_with(&prefix));
    }
    cx.print(&list, || table(&list, false))?;
    Ok(ExitCode::SUCCESS)
}
