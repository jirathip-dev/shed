use std::io::{self, BufRead as _, Write as _};

use serde_json::Value;
use shed_server::proto::{Event, EventsParams, Method};
use shed_server::runner::ExitCode;

use crate::args::label;
use crate::cx::{Cx, On};

pub const ON: On = On::Link;

#[derive(clap::Args)]
pub struct Args {
    #[arg(long = "label", value_name = "K=V", value_parser = label)]
    labels: Vec<(String, String)>,
    #[arg(long, value_name = "N")]
    since: Option<u64>,
}

pub fn run(cx: &Cx<'_>, args: Args) -> anyhow::Result<ExitCode> {
    let server = &cx.server;
    let labels = args.labels.into_iter().collect();
    let mut params = EventsParams {
        labels,
        since: args.since,
    };
    let mut client = server.connect(None)?;
    let _: Value = client.call(Method::Events, &params)?;
    let mut out = io::stdout().lock();
    let mut lines = client.split().0.lines();
    loop {
        let Some(line) = lines.next() else {
            let Some(Ok(mut client)) = server.on.is_none().then(|| server.connect(None)) else {
                return Ok(ExitCode::SUCCESS);
            };
            let _: Value = client.call(Method::Events, &params)?;
            lines = client.split().0.lines();
            continue;
        };
        let mut line = line?;
        let event = serde_json::from_str::<Event>(&line).ok();
        params.since = event.as_ref().map(|e| e.seq).or(params.since);
        if let Some(m) = &server.on
            && let Some(mut event) = event
        {
            let prefix = format!("{}/", m.name);
            if server.linked && !event.name.starts_with(&prefix) {
                continue;
            }
            if !server.linked {
                event.name.insert_str(0, &prefix);
                line = serde_json::to_string(&event)?;
            }
        }
        writeln!(out, "{line}")?;
        out.flush()?;
    }
}
