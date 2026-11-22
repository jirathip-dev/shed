use std::path::Path;

use crate::service::{Checks, Row};

pub(super) fn rows(cx: &Checks<'_>) -> Vec<Row> {
    let log = &cx.commands.log;
    let log = match log.strip_prefix("tail -f ") {
        Some(log) if !Path::new(log).exists() => {
            Err(format!("{log} not written yet; {}", cx.fix))
        }
        Some(log) => Ok(log.to_owned()),
        None => Ok(log.clone()),
    };
    vec![Row::of("log", log)]
}
