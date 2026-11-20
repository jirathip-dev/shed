use std::path::PathBuf;

use clap::Subcommand;
use shed_server::proto::{Method, OutlineParams, OutlineResult, Span};
use shed_server::search;

use crate::{Server, print};

#[derive(Subcommand)]
pub enum CodeCommand {
    Outline { file: PathBuf },
}

pub fn code(server: &Server, json: bool, command: CodeCommand) -> anyhow::Result<()> {
    let CodeCommand::Outline { file } = command;
    let params = OutlineParams {
        path: server.resolve(file.clone())?,
        machine: server.machine(),
    };
    let found: OutlineResult = server.connect(None)?.call(Method::CodeOutline, &params)?;
    print(json, &found, || {
        let path = file.display().to_string();
        search::text::outline(&OutlineResult {
            path,
            ..found.clone()
        })
    })
}

pub fn span(s: &str) -> Result<Span, String> {
    let (start, end) = s.split_once('-').unwrap_or((s, s));
    let line = |n: &str| n.parse::<usize>().ok().filter(|&n| n > 0);
    match (line(start), line(end)) {
        (Some(start), Some(end)) if start <= end => Ok(Span { start, end }),
        _ => Err(format!("{s:?} is not N or N-M, 1-based, N <= M")),
    }
}
