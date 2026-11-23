use std::fmt::Write as _;

use anyhow::Context as _;
use clap::Subcommand;
use shed_client::Calls as _;
use shed_server::proto::{Report, ReportParams, when};
use shed_server::runner::ExitCode;

use crate::cx::{Cx, On};

pub const ON: On = On::Link;

#[derive(clap::Args)]
pub struct Args {
    #[command(subcommand)]
    action: Action,
}

#[derive(Subcommand)]
enum Action {
    Report {
        #[arg(long, default_value = "7d", value_name = "SPAN")]
        since: String,
    },
}

fn text(report: &Report) -> String {
    let mut out = format!("build {} since {}\n", report.build, report.since);
    for f in &report.flows {
        let fired: u64 = f.fired.values().sum();
        let flow = format!("{:?} ({:?})", f.flow, f.mode).to_lowercase();
        let (never, odd, stuck) = (&f.never_seen, &f.unexpected, &f.stuck);
        let counts = format!("never seen {never:?}, unexpected {odd:?}, stuck {stuck:?}");
        writeln!(out, "{flow}: {} subjects, {fired} rows fired, {counts}", f.subjects).ok();
        for c in &f.disagreements {
            let (kind, from, to) = (format!("{:?}", c.kind).to_lowercase(), c.first, c.last);
            let what = format!("table {}, code {}: {} times", c.table, c.code, c.count);
            writeln!(out, "  {kind} at {}: {what}, {from} to {to}, e.g. {}", c.row, c.sample).ok();
        }
    }
    out
}

pub fn run(cx: &Cx<'_>, args: Args) -> anyhow::Result<ExitCode> {
    let Action::Report { since } = args.action;
    let span = when(&format!("in {since}"), 0).context("--since takes a span: 7d, 12h, 30m")?;
    let params = ReportParams {
        since: Some(shed_server::clock::unix_now().saturating_sub(span)),
        machine: cx.server.machine(),
    };
    let report = cx.server.connect(None)?.flow_report(&params)?;
    cx.print(&report, || text(&report))?;
    Ok(ExitCode::SUCCESS)
}
