//! The inside socket: what a sandboxed command may call, in a folder of its own beside the state file.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::proto::Method;
use crate::{Shared, links};

pub(crate) const METHODS: [Method; 4] = [
    Method::Hello,
    Method::Heartbeat,
    Method::State,
    Method::Emit,
];

pub(crate) fn socket(state: &Path) -> PathBuf {
    state.with_file_name("inside").join("inside.sock")
}

pub(crate) fn listen(
    registry: &Shared,
    links: &Arc<links::Links>,
    state: &Path,
) -> std::io::Result<()> {
    let at = socket(state);
    let folder = at.parent().unwrap_or(state);
    std::os::unix::fs::DirBuilderExt::mode(std::fs::DirBuilder::new().recursive(true), 0o700)
        .create(folder)?;
    let listener = crate::serve::bind(&at)?;
    let (registry, links) = (Arc::clone(registry), Arc::clone(links));
    tokio::spawn(async move { crate::serve::accept(&listener, &registry, &links, true).await });
    Ok(())
}
