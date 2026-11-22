use shed_server::runner::ExitCode;

use crate::args::label;
use crate::cx::{Cx, On};

pub const ON: On = On::Ssh;

#[derive(clap::Args)]
pub struct Args {
    #[arg(long = "label", value_name = "K=V", value_parser = label)]
    labels: Vec<(String, String)>,
}

pub fn run(cx: &Cx<'_>, args: Args) -> anyhow::Result<ExitCode> {
    crate::watch::watch(&cx.server, args.labels.into_iter().collect())?;
    Ok(ExitCode::SUCCESS)
}
