use shed_client::Error;

use crate::service::{Checks, Row};

pub(super) fn rows(cx: &Checks<'_>) -> Vec<Row> {
    let socket = cx.paths.socket.display();
    let socket = match &cx.server {
        Err(e) if !matches!(e, Error::Version { .. }) => Err(format!("{socket}: {e}; {}", cx.fix)),
        _ => Ok(socket.to_string()),
    };
    vec![Row::of("socket", socket)]
}
