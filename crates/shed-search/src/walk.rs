use std::fs;
use std::io::Read as _;
use std::path::Path;

use crate::{Error, Limit, Over};

pub(crate) fn read(path: &Path, max: u64) -> Result<Vec<u8>, Error> {
    let io = |e: std::io::Error| Error::Io(format!("{}: {e}", path.display()));
    let file = fs::File::open(path).map_err(io)?;
    if file.metadata().map_err(io)?.len() > max {
        let at = path.display().to_string();
        return Err(Error::Limit(Over {
            limit: Limit::Bytes,
            max,
            at,
        }));
    }
    let mut bytes = Vec::new();
    file.take(max).read_to_end(&mut bytes).map_err(io)?;
    Ok(bytes)
}
