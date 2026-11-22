use crate::service::{Checks, Row};

pub(super) fn rows(cx: &Checks<'_>) -> Vec<Row> {
    let bad = shed_server::providers::Catalog::load(&cx.paths.providers()).1;
    let row = |e| Row::of("provider", Err(format!("{e}; skipped")));
    bad.iter().map(row).collect()
}
