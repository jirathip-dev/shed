use std::path::PathBuf;

use clap::{Args, Subcommand};
use shed_server::proto::{GrepParams, GrepResult, Method, OutlineParams, OutlineResult, Span};
use shed_server::search;

use crate::{Server, print};

#[derive(Subcommand)]
pub enum CodeCommand {
    Grep(GrepArgs),
    Outline { file: PathBuf },
}

#[derive(Args)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "one flag per grep option, as on the wire"
)]
pub struct GrepArgs {
    #[arg(short = 'F', long)]
    fixed: bool,
    #[arg(short, long)]
    ignore_case: bool,
    #[arg(short, long)]
    word: bool,
    #[arg(long)]
    count: bool,
    #[arg(long)]
    hidden: bool,
    #[arg(short, long, value_name = "G")]
    glob: Vec<String>,
    pattern: String,
    path: Option<PathBuf>,
}

pub fn code(server: &Server, json: bool, command: CodeCommand) -> anyhow::Result<()> {
    let file = match command {
        CodeCommand::Grep(args) => return grep(server, json, args),
        CodeCommand::Outline { file } => file,
    };
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

fn grep(server: &Server, json: bool, a: GrepArgs) -> anyhow::Result<()> {
    let params = GrepParams {
        root: server.resolve(a.path.unwrap_or_else(|| PathBuf::from(".")))?,
        pattern: a.pattern,
        fixed: a.fixed,
        ignore_case: a.ignore_case,
        word: a.word,
        count: a.count,
        hidden: a.hidden,
        glob: a.glob,
        machine: server.machine(),
    };
    let found: GrepResult = server.connect(None)?.call(Method::CodeGrep, &params)?;
    print(json, &found, || search::text::grep(&found))
}

pub fn span(s: &str) -> Result<Span, String> {
    let (start, end) = s.split_once('-').unwrap_or((s, s));
    let line = |n: &str| n.parse::<usize>().ok().filter(|&n| n > 0);
    match (line(start), line(end)) {
        (Some(start), Some(end)) if start <= end => Ok(Span { start, end }),
        _ => Err(format!("{s:?} is not N or N-M, 1-based, N <= M")),
    }
}
