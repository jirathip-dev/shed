use crate::service::{Checks, Row};

pub(super) fn rows(_: &Checks<'_>) -> Vec<Row> {
    let memory = shed_server::limit::mechanism();
    vec![Row::of("memory", Ok(memory.to_owned()))]
}
