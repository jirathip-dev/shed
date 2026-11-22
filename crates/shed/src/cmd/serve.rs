use std::time::Duration;

use anyhow::Context as _;
use shed_server::handover;
use shed_server::runner::ExitCode;

use crate::cx::{Cx, On, serve_args, serve_detached};
use crate::out;

pub const ON: On = On::No;

#[derive(clap::Args)]
pub struct Args {
    #[arg(long)]
    detach: bool,
    #[arg(long, value_name = "S", default_value_t = 2.0)]
    sample: f64,
    #[arg(long, hide = true)]
    inherit: bool,
    #[arg(long, hide = true)]
    preflight: bool,
}

pub fn run(cx: &Cx<'_>, Args { detach, sample, inherit, preflight }: Args) -> anyhow::Result<ExitCode> {
    let (paths, dir) = (cx.paths, cx.dir);
    if preflight {
        out::line(handover::preflight(paths)?);
        return Ok(ExitCode::SUCCESS);
    }
    if detach {
        serve_detached(paths, dir, Some(sample))?;
        return Ok(ExitCode::SUCCESS);
    }
    let every = Duration::try_from_secs_f64(sample)
        .ok()
        .filter(|d| !d.is_zero())
        .context("--sample must be a positive number of seconds")?;
    let again = handover::Restart {
        exe: std::env::current_exe()?,
        inherit: [serve_args(dir, Some(sample)), vec!["--inherit".into()]].concat(),
        preflight: [serve_args(dir, None), vec!["--preflight".into()]].concat(),
    };
    shed_server::serve(paths, every, inherit, again).context("shed serve")?;
    Ok(ExitCode::SUCCESS)
}
