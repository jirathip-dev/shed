use crate::service::{Checks, Row, on_disk};

pub(super) fn rows(cx: &Checks<'_>) -> Vec<Row> {
    vec![Row::of("job", on_disk(&cx.job, &cx.status))]
}
