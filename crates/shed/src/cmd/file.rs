use std::path::PathBuf;

use anyhow::bail;
use serde_json::Value;
use shed_server::proto::{FileParams, Method, Span};
use shed_server::runner::ExitCode;

use crate::args::span;
use crate::cx::{Cx, On, stdin_bytes};
use crate::out;

pub const ON: On = On::Link;

#[derive(clap::Args)]
pub struct Args {
    #[arg(value_parser = ["read", "write"])]
    action: String,
    path: PathBuf,
    #[arg(long, value_name = "N-M", value_parser = span)]
    lines: Option<Span>,
}

pub fn run(cx: &Cx<'_>, args: Args) -> anyhow::Result<ExitCode> {
    let server = &cx.server;
    let write = args.action == "write";
    if write && args.lines.is_some() {
        bail!("--lines is for file read");
    }
    let params = FileParams {
        path: server.resolve(args.path)?,
        data: write.then(stdin_bytes).transpose()?,
        lines: args.lines,
        machine: server.machine(),
    };
    let method = match params.data {
        Some(_) => Method::FileWrite,
        None => Method::FileRead,
    };
    let done: Value = server.connect(None)?.call(method, &params)?;
    let bytes: Vec<u8> = serde_json::from_value(done.clone()).unwrap_or_default();
    out::raw(cx.json, &done, &bytes, &[])?;
    Ok(ExitCode::SUCCESS)
}
