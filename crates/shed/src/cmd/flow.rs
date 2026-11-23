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
    let join = |list: Vec<String>| if list.is_empty() { "-".to_owned() } else { list.join(" ") };
    let pairs = |m: &std::collections::BTreeMap<String, u64>| {
        join(m.iter().map(|(k, n)| format!("{k}={n}")).collect())
    };
    for f in &report.flows {
        let fired: u64 = f.fired.values().sum();
        let (flow, mode) = (format!("{:?}", f.flow), format!("{:?}", f.mode));
        writeln!(
            out,
            "{flow} ({mode}): {} subjects, {fired} rows fired; never seen: {}; unexpected: {}; stuck: {}",
            f.subjects,
            join(f.never_seen.clone()),
            pairs(&f.unexpected),
            pairs(&f.stuck),
        )
        .ok();
        for c in &f.disagreements {
            writeln!(
                out,
                "  {:?} at {}: table {}, code {}: {} times, {} to {}, e.g. {}",
                c.kind, c.row, c.table, c.code, c.count, c.first, c.last, c.sample
            )
            .ok();
        }
    }
    out.to_lowercase()
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
