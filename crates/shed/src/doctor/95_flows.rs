use shed_client::Calls as _;
use shed_server::proto::{Report, ReportParams};

use crate::service::{Checks, Mark, Row};

pub(super) fn rows(cx: &Checks<'_>) -> Vec<Row> {
    let since = Some(shed_server::clock::unix_now().saturating_sub(86_400));
    let params = ReportParams { since, machine: None };
    let report = cx.server.as_ref().map(|c| c.flow_report(&params));
    let count = |r: Report| -> u64 {
        let classes = r.flows.into_iter().flat_map(|f| f.disagreements);
        classes.map(|c| c.count).sum()
    };
    let lines = report.map(|r| r.map(count));
    let (mark, text) = match lines {
        Ok(Ok(0)) => (Mark::Ok, "no shadow disagreements in 24 h".to_owned()),
        Ok(Ok(n)) => (Mark::Note, format!("{n} shadow disagreements in 24 h; see shed flow report")),
        Ok(Err(e)) => (Mark::Note, e.to_string()),
        Err(_) => (Mark::Note, "no server to ask".to_owned()),
    };
    vec![Row { mark, what: "flows", text }]
}
