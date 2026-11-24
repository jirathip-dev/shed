use std::fmt::Write as _;
use std::io::Read as _;

use clap::Subcommand;
use shed_client::Calls as _;
use shed_server::proto::SavedSecret;
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
    Set { name: String },
    Ls,
    Rm { name: String },
}

fn table(list: &[SavedSecret]) -> String {
    let mut out = String::from("NAME  SAVED\n");
    for s in list {
        let at = s.saved_at.map_or("-".to_owned(), |t| t.to_string());
        writeln!(out, "{}  {at}", s.name).ok();
    }
    out
}

pub fn run(cx: &Cx<'_>, args: Args) -> anyhow::Result<ExitCode> {
    let shed = cx.server.connect(None)?;
    match args.action {
        Action::Set { name } => {
            let mut value = String::new();
            std::io::stdin().read_to_string(&mut value)?;
            if value.ends_with('\n') {
                value.pop();
            }
            shed.secret_set(&name, value)?;
            out::line(format_args!("{name} saved"));
        }
        Action::Ls => {
            let list = shed.secrets()?;
            cx.print(&list, || table(&list))?;
        }
        Action::Rm { name } => {
            shed.secret_rm(&name)?;
            out::line(format_args!("{name} removed"));
        }
    }
    Ok(ExitCode::SUCCESS)
}
