use std::fmt::Write as _;

use clap::Subcommand;
use shed_client::Calls as _;
use shed_server::proto::ProxyKey;
use shed_server::runner::ExitCode;

use crate::cx::{Cx, On};
use crate::out;

pub const ON: On = On::No;

#[derive(clap::Args)]
pub struct Args {
    #[command(subcommand)]
    action: Action,
}

#[derive(Subcommand)]
enum Action {
    Keys,
    Revoke { id: String },
}

fn table(list: &[ProxyKey]) -> String {
    let mut out = String::from("ID  STATE  TERMINAL  ACCOUNT  CREATED\n");
    for k in list {
        let state = format!("{:?}", k.state).to_lowercase();
        let account = format!("{}/{}", k.provider, k.account);
        writeln!(out, "{}  {state}  {}  {account}  {}", k.id, k.terminal, k.created).ok();
    }
    out
}

pub fn run(cx: &Cx<'_>, args: Args) -> anyhow::Result<ExitCode> {
    let shed = cx.server.connect(None)?;
    match args.action {
        Action::Keys => {
            let list = shed.proxy_keys()?;
            cx.print(&list, || table(&list))?;
        }
        Action::Revoke { id } => {
            shed.proxy_revoke(&id)?;
            out::line(format_args!("{id} revoked"));
        }
    }
    Ok(ExitCode::SUCCESS)
}
