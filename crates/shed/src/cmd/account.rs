use std::fmt::Write as _;
use std::io::BufRead as _;
use std::path::PathBuf;

use clap::Subcommand;
use shed_client::Calls as _;
use shed_server::proto::{Account, AccountAdd, SendParams, State};
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
    Add {
        provider: String,
        name: String,
        #[arg(long, value_name = "DIR")]
        folder: Option<PathBuf>,
        #[arg(long, value_name = "FILE")]
        token_file: Option<PathBuf>,
    },
    Ls {
        provider: Option<String>,
    },
    Login {
        provider: String,
        name: String,
        #[arg(long)]
        token: bool,
    },
    Status {
        provider: String,
        name: String,
    },
    Rm {
        provider: String,
        name: String,
    },
    Use {
        provider: String,
        #[arg(required_unless_present = "clear", conflicts_with = "clear")]
        name: Option<String>,
        #[arg(long)]
        clear: bool,
    },
}

fn table(list: &[Account]) -> String {
    let mut out = String::from("ACCOUNT  STATE  WHO  USAGE  TOKEN  LINE\n");
    for a in list {
        let usage = a.usage.iter().flat_map(|u| &u.windows);
        let usage = usage.map(|w| format!("{} {}%", w.id, w.used_pct));
        let balance = a.balance.iter().map(|b| format!("{} {}", b.total, b.currency));
        let usage: Vec<_> = usage.chain(balance).collect();
        let token = a.token.map_or("-".to_owned(), |t| match t.expires_at {
            Some(at) => format!("until {at}"),
            None => "saved".to_owned(),
        });
        let state = format!("{:?}", a.state).to_lowercase();
        let who = a.who.as_deref().unwrap_or("-");
        let preferred = if a.preferred { " (preferred)" } else { "" };
        let account = format!("{}/{}{preferred}", a.provider, a.name);
        let usage = usage.join(",");
        writeln!(out, "{account}  {state}  {who}  {usage}  {token}  {}", a.line).ok();
    }
    out
}

pub fn run(cx: &Cx<'_>, args: Args) -> anyhow::Result<ExitCode> {
    let shed = cx.server.connect(None)?;
    let account = match args.action {
        Action::Ls { provider } => {
            let list = shed.accounts(provider.as_deref())?;
            cx.print(&list, || table(&list))?;
            return Ok(ExitCode::SUCCESS);
        }
        Action::Rm { provider, name } => {
            shed.account_rm(&provider, &name)?;
            out::line(format_args!("{provider}/{name} removed"));
            return Ok(ExitCode::SUCCESS);
        }
        Action::Use { provider, name, .. } => {
            let moves = shed.account_use(&provider, name.as_deref())?;
            cx.print(&moves, || {
                let mut out = String::new();
                for t in &moves.moved {
                    writeln!(out, "{t}  moved").ok();
                }
                for s in &moves.stayed {
                    writeln!(out, "{}  stayed: {}", s.terminal, s.why).ok();
                }
                out
            })?;
            return Ok(ExitCode::SUCCESS);
        }
        Action::Add {
            provider,
            name,
            folder,
            token_file,
        } => {
            let folder = folder.map(|f| cx.server.resolve(f)).transpose()?;
            let token_file = token_file.map(|f| cx.server.resolve(f)).transpose()?;
            let add = AccountAdd {
                provider,
                name,
                folder,
                token_file,
            };
            shed.account_add(&add)?
        }
        Action::Status { provider, name } => shed.account_status(&provider, &name)?,
        Action::Login {
            provider,
            name,
            token,
        } => {
            let terminal = shed.account_login(&provider, &name, token)?.terminal;
            login(cx, &terminal)?;
            shed.account_status(&provider, &name)?
        }
    };
    cx.print(&account, || table(std::slice::from_ref(&account)))?;
    Ok(ExitCode::SUCCESS)
}

fn login(cx: &Cx<'_>, terminal: &str) -> anyhow::Result<()> {
    let typing = cx.server.connect(None)?;
    let name = terminal.to_owned();
    std::thread::spawn(move || {
        for text in std::io::stdin().lock().lines().map_while(Result::ok) {
            let name = name.clone();
            let send = SendParams {
                name,
                text,
                ..SendParams::default()
            };
            if typing.send(&send).is_err() {
                return;
            }
        }
    });
    let shed = cx.server.connect(None)?;
    let mut shown = String::new();
    loop {
        let waited = shed.wait_state(terminal, &["done"], 2.0)?;
        let screen = shed.screen(terminal)?;
        if screen != shown {
            out::line(format_args!("\x1b[H\x1b[2J{screen}"));
            shown = screen;
        }
        if waited.terminal.state == State::Exited {
            return Ok(());
        }
    }
}
