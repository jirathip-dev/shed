use shed_server::runner::ExitCode;

use crate::cx::{Cx, On};
use crate::out;
use crate::service::{self, Mark};

pub const ON: On = On::No;

#[derive(clap::Args)]
pub struct Args {}

pub fn run(cx: &Cx<'_>, _: Args) -> anyhow::Result<ExitCode> {
    let rows = service::doctor(cx.paths)?;
    for row in &rows {
        let mark = match row.mark {
            Mark::Ok => "ok",
            Mark::Fail => "FAIL",
            Mark::Note => "--",
        };
        out::line(format_args!("{mark:<5} {:<8} {}", row.what, row.text));
    }
    let ok = rows.iter().all(|r| !matches!(r.mark, Mark::Fail));
    Ok(if ok {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}
