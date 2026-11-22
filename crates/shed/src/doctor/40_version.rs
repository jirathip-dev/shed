use shed_client::Error;
use shed_server::proto::{BUILD, LONG_VERSION};

use crate::service::{Checks, Row};

pub(super) fn rows(cx: &Checks<'_>) -> Vec<Row> {
    let version = match &cx.server {
        Ok(c) if c.server().build == BUILD => Ok(format!("shed {LONG_VERSION}")),
        Ok(c) => Err(format!(
            "binary {BUILD}, server {}; fix: shed install",
            c.server().label()
        )),
        Err(e @ Error::Version { .. }) => Err(format!("{e}; fix: shed install")),
        Err(_) => Err("no server to ask".into()),
    };
    vec![Row::of("version", version)]
}
