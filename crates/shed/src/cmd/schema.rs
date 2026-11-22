use shed_server::runner::ExitCode;

use crate::cx::{Cx, On};
use crate::out;

pub const ON: On = On::No;

#[derive(clap::Args)]
pub struct Args {}

pub fn run(_: &Cx<'_>, _: Args) -> ExitCode {
    out::line(format_args!("{:#}", shed_server::proto::openrpc()));
    ExitCode::SUCCESS
}
