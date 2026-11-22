use crate::service::{Checks, Row, UPDATE, on_disk};

pub(super) fn rows(cx: &Checks<'_>) -> Vec<Row> {
    let update = &cx.update;
    let update = on_disk(update, &update.status()).and_then(|_| match crate::update::last(cx.paths) {
        Some(why) if why.starts_with("rejected") || why.starts_with("deferred") => {
            Err(format!("{why}; see docs/self-update.md"))
        }
        why => Ok(format!(
            "{UPDATE}: {}",
            why.as_deref().unwrap_or("no pass yet")
        )),
    });
    vec![Row::of("update", update)]
}
