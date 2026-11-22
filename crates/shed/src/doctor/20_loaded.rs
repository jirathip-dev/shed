use farrier_service::Status;

use crate::service::{Checks, LABEL, Row};

pub(super) fn rows(cx: &Checks<'_>) -> Vec<Row> {
    let outside = cx.server.is_ok().then(|| cx.paths.socket.display().to_string());
    let fix = &cx.commands.load;
    let loaded = match (&cx.status, outside) {
        (Ok(s), _) if s.loaded && s.refused.is_none() => Ok(LABEL.into()),
        (
            Ok(Status {
                refused: Some(why), ..
            }),
            _,
        ) => Err(format!("{why}; fix: shed install")),
        (_, Some(server)) => Err(format!(
            "a server outside {LABEL} answers on {server}; fix: stop that server, then {fix}"
        )),
        _ => Err(format!("not loaded; fix: {fix}")),
    };
    vec![Row::of("loaded", loaded)]
}
