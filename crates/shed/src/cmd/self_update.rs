use std::fs;
use std::io::IsTerminal as _;

use anyhow::Context as _;
use serde_json::json;
use shed_client::Conn;
use shed_server::clock;
use shed_server::proto::BUILD;
use shed_server::runner::ExitCode;

use crate::cx::{Cx, On};
use crate::update::{last, pass};

pub const ON: On = On::No;

#[derive(clap::Args)]
pub struct Args {
    #[arg(long, conflicts_with_all = ["pause", "resume"])]
    dry_run: bool,
    #[arg(long, conflicts_with = "resume")]
    pause: bool,
    #[arg(long)]
    resume: bool,
}

pub fn run(cx: &Cx<'_>, Args { dry_run, pause, resume }: Args) -> anyhow::Result<ExitCode> {
    let paths = cx.paths;
    let dir = paths.state.with_file_name("update");
    fs::create_dir_all(&dir).with_context(|| dir.display().to_string())?;
    let marker = dir.join("paused");
    if pause {
        fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&marker)?;
        crate::out::line("paused");
        return Ok(ExitCode::SUCCESS);
    } else if resume {
        fs::remove_file(&marker).ok();
        crate::out::line("resumed");
        return Ok(ExitCode::SUCCESS);
    }
    let lock = fs::File::create(dir.join("lock"))?;
    if lock.try_lock().is_err() {
        crate::out::line("another self-update pass runs");
        return Ok(ExitCode::SUCCESS);
    }
    let exe = std::env::current_exe().and_then(fs::canonicalize);
    let exe = exe.context("this binary's path")?;
    let dry = dry_run;
    if dry {
        let serve = Conn::connect(&paths.socket).map(|c| c.server().build.clone());
        let serve = serve.unwrap_or_else(|_| "not running".to_owned());
        crate::out::line(format_args!("current  {BUILD}, serve {serve}"));
    }
    let (ok, line) = match pass(paths, &dir, &exe, dry) {
        Ok(line) => (true, line),
        Err(e) => (false, format!("rejected: {e:#}; the live binary stays")),
    };
    if dry || std::io::stdout().is_terminal() || last(paths).is_none_or(|why| why != line) {
        crate::out::line(&line);
    }
    if !dry {
        let status = json!({"at": clock::unix_now(), "current": BUILD, "ok": ok, "why": line});
        fs::write(dir.join("status.json"), status.to_string())?;
    }
    Ok(if ok {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}
