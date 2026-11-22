use shed_client::Conn;
use shed_server::Paths;
use shed_server::runner::ExitCode;

use crate::cx::{Cx, On};
use crate::out;

pub const ON: On = On::No;

#[derive(clap::Args)]
pub struct Args {}

pub fn run(cx: &Cx<'_>, _: Args) -> ExitCode {
    out::line(format_args!("shed {}", shed_server::proto::LONG_VERSION));
    if let Some(why) = other_build(cx.paths) {
        out::err(why);
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}

fn other_build(paths: &Paths) -> Option<String> {
    let server = match Conn::connect(&paths.socket) {
        Ok(client) => client.server().clone(),
        Err(e @ shed_client::Error::Version { .. }) => return Some(e.to_string()),
        Err(_) => return None,
    };
    let ours = shed_server::proto::BUILD;
    let theirs = server.label();
    (server.build != ours)
        .then(|| format!("shed serve runs build {theirs}, this is {ours}; restart shed serve"))
}
