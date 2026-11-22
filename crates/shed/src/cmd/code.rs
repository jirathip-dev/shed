use std::path::PathBuf;

use clap::Subcommand;
use shed_server::proto::{GrepParams, GrepResult, Method, OutlineParams, OutlineResult};
use shed_server::runner::ExitCode;
use shed_server::search;

use crate::cx::{Cx, On};

pub const ON: On = On::Link;

#[derive(clap::Args)]
pub struct Args {
    #[command(subcommand)]
    command: Action,
}

#[derive(Subcommand)]
enum Action {
    Grep(GrepArgs),
    Outline { file: PathBuf },
}

#[derive(clap::Args)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "one flag per grep option, as on the wire"
)]
struct GrepArgs {
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

pub fn run(cx: &Cx<'_>, args: Args) -> anyhow::Result<ExitCode> {
    let server = &cx.server;
    let file = match args.command {
        Action::Grep(args) => return grep(cx, args),
        Action::Outline { file } => file,
    };
    let params = OutlineParams {
        path: server.resolve(file.clone())?,
        machine: server.machine(),
    };
    let found: OutlineResult = server.connect(None)?.call(Method::CodeOutline, &params)?;
    cx.print(&found, || {
        let path = file.display().to_string();
        search::text::outline(&OutlineResult {
            path,
            ..found.clone()
        })
    })?;
    Ok(ExitCode::SUCCESS)
}

fn grep(cx: &Cx<'_>, a: GrepArgs) -> anyhow::Result<ExitCode> {
    let server = &cx.server;
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
    cx.print(&found, || search::text::grep(&found))?;
    Ok(ExitCode::SUCCESS)
}
