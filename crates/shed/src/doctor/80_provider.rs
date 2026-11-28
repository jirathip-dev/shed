use shed_client::Calls as _;
use shed_server::proto::EventKind;
use shed_server::providers::{Catalog, config_folder};
use shed_server::stale_lock;

use crate::service::{Checks, Row};

pub(super) fn rows(cx: &Checks<'_>) -> Vec<Row> {
    let (catalog, bad) = Catalog::load(&cx.paths.providers());
    let row = |e| Row::of("provider", Err(format!("{e}; skipped")));
    let mut rows: Vec<_> = bad.iter().map(row).collect();
    let accounts = (cx.server.as_ref().ok()).and_then(|s| s.accounts(None).ok());
    let now = shed_server::clock::unix_now();
    for record in catalog.iter().map(|e| &e.provider) {
        let named = (accounts.iter().flatten())
            .filter(|a| a.provider == record.id)
            .filter_map(|a| a.folder.clone());
        for folder in config_folder(record, &[]).into_iter().chain(named) {
            if let Some(EventKind::LockCleared { path, age_secs, .. }) =
                stale_lock::found(record, &folder, now)
            {
                let (id, path) = (&record.id, path.display());
                let why = format!("{id}: stale empty {path}, {age_secs} s old; the next start removes it, or rm it");
                rows.push(Row::of("provider", Err(why)));
            }
        }
    }
    rows
}
