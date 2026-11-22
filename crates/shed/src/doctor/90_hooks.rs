use std::path::Path;

use crate::service::{Checks, Mark, Row};

pub(super) fn rows(cx: &Checks<'_>) -> Vec<Row> {
    let home = shed_server::config::var("HOME").unwrap_or_default();
    let dir = cx.paths.providers();
    let hooks = match shed_server::hooks::setup(Path::new(&home), &dir, &[], false, true) {
        Ok(hooks) => hooks,
        Err(e) => return vec![Row::of("hooks", Err(e.to_string()))],
    };
    let row = |f: &shed_server::hooks::HookFile| {
        let (agent, file) = (&f.agent, f.file.display());
        let fix = f.after.as_ref().map_or("", |_| " lacks them; fix: shed setup");
        let mark = if fix.is_empty() { Mark::Ok } else { Mark::Note };
        let text = format!("{agent}: {file}{fix}");
        Row {
            mark,
            what: "hooks",
            text,
        }
    };
    hooks.iter().map(row).collect()
}
