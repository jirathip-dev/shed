use anyhow::Context as _;
use shed_client::Calls as _;
use shed_server::proto::{Children, State, WaitOutcome, WaitParams, WaitResult};
use shed_server::runner::ExitCode;

use crate::args::children;
use crate::cx::{Cx, On};

pub const ON: On = On::Link;

#[derive(clap::Args)]
pub struct Args {
    #[arg(required_unless_present = "children")]
    name: Option<String>,
    #[arg(long, value_name = "all|any", value_parser = children, conflicts_with_all = ["until", "output"])]
    children: Option<Children>,
    #[arg(long, num_args = 1.., value_name = "STATE",
          value_parser = ["working", "blocked", "done", "error", "idle", "running",
                          "waiting-approval", "rate-limited", "hibernated"])]
    until: Vec<String>,
    #[arg(long, value_parser = ["permission", "question", "auth"])]
    kind: Option<String>,
    #[arg(long, value_name = "RE")]
    output: Option<String>,
    #[arg(long, value_name = "S")]
    timeout: Option<f64>,
}

pub fn run(cx: &Cx<'_>, args: Args) -> anyhow::Result<ExitCode> {
    let server = &cx.server;
    let name = match args.name {
        Some(name) => name,
        None => shed_server::config::var("SHED")
            .context("no terminal given and $SHED is unset (it is set inside a terminal)")?,
    };
    let params = WaitParams {
        name: server.name(name),
        children: args.children,
        until: args.until,
        kind: args.kind,
        output: args.output,
        timeout: args.timeout,
    };
    let started = shed_server::clock::now();
    let done: WaitResult = server.again(|c| {
        let gone = shed_server::clock::now()
            .duration_since(started)
            .as_secs_f64();
        let timeout = params.timeout.map(|t| (t - gone).max(0.0));
        c.wait(&WaitParams {
            timeout,
            ..params.clone()
        })
    })?;
    let t = &done.terminal;
    let state = t.reported.as_ref().map(|r| &r.state);
    cx.print(&done, || match (done.outcome, t.exit) {
        (WaitOutcome::Matched, _) if params.children == Some(Children::All) => {
            format!("{} no child working\n", t.name)
        }
        (WaitOutcome::Matched, code) if params.children.is_some() && t.state == State::Exited => {
            format!(
                "{} exited{}\n",
                t.name,
                code.map(|c| format!(" {c}")).unwrap_or_default()
            )
        }
        (WaitOutcome::Matched, _) if params.children.is_some() => {
            format!("{} {}\n", t.name, state.map_or("running", String::as_str))
        }
        (WaitOutcome::Matched, _) => match state.filter(|s| params.until.contains(s)) {
            Some(state) => format!("{} {state}\n", t.name),
            None => format!("{} output\n", t.name),
        },
        (WaitOutcome::Exited, Some(code)) => format!("{} exited {code}\n", t.name),
        (WaitOutcome::Exited, None) => format!("{} exited\n", t.name),
        (WaitOutcome::Timeout, _) => format!("{} timeout\n", t.name),
    })?;
    Ok(ExitCode::from(match done.outcome {
        WaitOutcome::Matched => 0,
        WaitOutcome::Exited => 1,
        WaitOutcome::Timeout => 124,
    }))
}
