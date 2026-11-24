use crate::service::{Checks, Mark, Row};

pub(super) fn rows(cx: &Checks<'_>) -> Vec<Row> {
    let problems = shed_server::secrets::problems(&cx.paths.state).into_iter();
    let row = |(fails, text)| Row {
        mark: if fails { Mark::Fail } else { Mark::Note },
        what: "secrets",
        text,
    };
    problems.map(row).collect()
}
