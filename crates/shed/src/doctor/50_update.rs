use crate::service::{Checks, Row, UPDATE, old_updater, on_disk};

pub(super) fn rows(cx: &Checks<'_>) -> Vec<Row> {
    let old: Vec<String> = (old_updater(&cx.update).iter())
        .map(|file| file.display().to_string())
        .collect();
    if !old.is_empty() {
        let fix = format!("{}: an old-named updater; fix: shed install", old.join(", "));
        return vec![Row::of("update", Err(fix))];
    }
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
