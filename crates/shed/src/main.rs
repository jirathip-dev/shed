//! The `shed` binary: argument parsing and wiring only.
#![forbid(unsafe_code)]

mod args;
mod attach;
mod cx;
mod out;
mod service;
mod skill;
mod table;
mod update;
mod watch;

mod cmd {
    include!(concat!(env!("OUT_DIR"), "/cmd.rs"));
}

use std::path::PathBuf;

use clap::Parser;
use shed_server::Paths;
use shed_server::runner::ExitCode;

use crate::cx::{Cx, Server};

#[derive(Parser)]
#[command(version = shed_server::proto::LONG_VERSION)]
struct Cli {
    #[arg(long, global = true)]
    dir: Option<PathBuf>,
    #[arg(long, global = true)]
    json: bool,
    #[arg(long, global = true, value_name = "MACHINE")]
    on: Option<String>,
    #[command(subcommand)]
    command: cmd::Command,
}

fn main() -> ExitCode {
    match run(Cli::parse()) {
        Ok(code) => code,
        Err(e) => {
            out::err(format_args!("shed: {e:#}"));
            ExitCode::FAILURE
        }
    }
}

fn run(cli: Cli) -> anyhow::Result<ExitCode> {
    let paths = match &cli.dir {
        Some(dir) => Paths::in_dir(dir),
        None => Paths::user()?,
    };
    let dir = cli.dir.as_deref();
    let server = Server::new(&paths, dir, cli.on, &cmd::on(&cli.command))?;
    let cx = Cx {
        paths: &paths,
        dir,
        json: cli.json,
        server,
    };
    cmd::run(&cx, cli.command)
}
