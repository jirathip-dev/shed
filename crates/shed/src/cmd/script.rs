use std::path::PathBuf;

use anyhow::Context as _;
use serde_json::Value;
use shed_server::config;
use shed_server::proto::{Caller, Method, ScriptParams, ScriptResult};
use shed_server::runner::ExitCode;

use crate::cx::{Cx, On, stdin_bytes};
use crate::out;

pub const ON: On = On::Link;

#[derive(clap::Args)]
#[command(group(clap::ArgGroup::new("program").required(true).args(["file", "stdin"])))]
pub struct Args {
    #[arg(short = 'f', value_name = "FILE")]
    file: Option<PathBuf>,
    #[arg(value_name = "-", value_parser = ["-"])]
    stdin: Option<String>,
    #[arg(long)]
    check: bool,
    #[arg(long, value_name = "S")]
    timeout: Option<f64>,
    #[arg(long, value_name = "R")]
    role: Option<String>,
}

pub fn run(cx: &Cx<'_>, args: Args) -> anyhow::Result<ExitCode> {
    let code = match &args.file {
        Some(file) => {
            std::fs::read_to_string(file).with_context(|| file.display().to_string())?
        }
        None => String::from_utf8(stdin_bytes()?).context("stdin is not UTF-8")?,
    };
    let server = &cx.server;
    let params = ScriptParams {
        code,
        timeout: args.timeout,
        machine: server.machine(),
        caller: Some(Caller {
            terminal: config::var("SHED"),
            cwd: std::env::current_dir().ok(),
            role: args.role,
        }),
        check: args.check,
    };
    let reply: ScriptResult = server.connect(None)?.call(Method::Script, &params)?;
    if cx.json {
        out::line(serde_json::to_string(&reply)?);
    } else {
        for log in &reply.logs {
            out::line(log);
        }
        match (&reply.error, reply.result.as_ref()) {
            (Some(e), _) => out::err(format_args!(
                "error: {} {}:{}: {}",
                e.name, e.line, e.column, e.message
            )),
            (None, _) if args.check => {}
            (None, Some(Value::String(text))) => out::line(text),
            (None, result) => out::line(serde_json::to_string_pretty(
                result.unwrap_or(&Value::Null),
            )?),
        }
        if reply.error.is_none() {
            out::err(format_args!("[{} calls, {} ms]", reply.calls, reply.ms));
        }
    }
    Ok(if reply.error.is_some() {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    })
}
