use std::path::PathBuf;

use shed_client::Calls as _;
use shed_server::proto::ExecParams;
use shed_server::runner::ExitCode;

use crate::cx::{Cx, On, stdin_bytes};
use crate::out;

pub const ON: On = On::Link;

#[derive(clap::Args)]
pub struct Args {
    #[arg(long)]
    cwd: Option<PathBuf>,
    #[arg(long, value_name = "S")]
    timeout: Option<f64>,
    #[arg(short = 'i')]
    stdin: bool,
    #[arg(last = true, required = true)]
    cmd: Vec<String>,
}

pub fn run(cx: &Cx<'_>, args: Args) -> anyhow::Result<ExitCode> {
    let server = &cx.server;
    let here = server.on.is_none().then(PathBuf::new);
    let params = ExecParams {
        cwd: args.cwd.or(here).map(|d| server.resolve(d)).transpose()?,
        timeout: args.timeout,
        stdin: args
            .stdin
            .then(stdin_bytes)
            .transpose()?
            .unwrap_or_default(),
        machine: server.machine(),
        cmd: args.cmd,
        ..ExecParams::default()
    };
    let done = server.connect(None)?.exec(&params)?;
    out::raw(cx.json, &done, &done.stdout, &done.stderr)?;
    Ok(ExitCode::from(u8::try_from(done.code).unwrap_or(1)))
}
