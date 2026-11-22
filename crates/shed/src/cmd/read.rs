use shed_client::Calls as _;
use shed_server::proto::ReadParams;
use shed_server::runner::ExitCode;

use crate::cx::{Cx, On};

pub const ON: On = On::Link;

#[derive(clap::Args)]
pub struct Args {
    name: String,
    #[arg(long)]
    lines: Option<usize>,
    #[arg(long, conflicts_with = "history")]
    screen: bool,
    #[arg(long)]
    history: bool,
    #[arg(long, conflicts_with_all = ["history", "screen"])]
    events: bool,
    #[arg(long, requires = "events")]
    before: Option<u64>,
}

pub fn run(cx: &Cx<'_>, args: Args) -> anyhow::Result<ExitCode> {
    let mut params = ReadParams {
        name: cx.server.name(args.name),
        lines: args.lines,
        history: args.history,
        events: args.events,
        before: args.before,
        ..ReadParams::default()
    };
    params.n = params.lines.filter(|_| params.events);
    let mut read = cx.server.connect(None)?.read(&params)?;
    if params.events && !cx.json {
        let mut preview = shed_server::lane::transcript::Preview::default();
        for e in read.events.drain(..) {
            preview.push(e.kind);
        }
        let (cols, _, color) = shed_server::lane::transcript::stdout_style();
        read.text = preview.draw(cols, color, usize::MAX).join("\n");
    }
    let end = if read.text.is_empty() { "" } else { "\n" };
    cx.print(&read, || format!("{}{end}", read.text))?;
    Ok(ExitCode::SUCCESS)
}
