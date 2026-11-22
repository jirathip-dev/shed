use std::fs;
use std::io::{self, BufRead as _, BufReader, Read as _, Write as _};
use std::os::unix::fs::{OpenOptionsExt as _, PermissionsExt as _};

use serde_json::Value;

use crate::call::{Reply, blocking, empty, io_error};
use crate::proto::{self, FileParams, Span};
use crate::runner;

pub(crate) async fn answer(p: FileParams) -> Reply {
    let path = p.path.display().to_string();
    let done = blocking(move || file(p)).await?;
    done.map_err(|e| io_error(format!("{path}: {e}")))
}

fn file(p: FileParams) -> io::Result<Value> {
    let Some(data) = p.data else {
        let file = fs::File::open(&p.path)?;
        let bytes = if let Some(span) = p.lines {
            lines(file, span)?
        } else {
            let mut bytes = Vec::new();
            let mut file = file.take(proto::REPLY_LIMIT.saturating_add(1));
            file.read_to_end(&mut bytes)?;
            bytes
        };
        within_limit(bytes.len())?;
        return serde_json::to_value(bytes).map_err(io::Error::other);
    };
    let mut temp = p.path.clone().into_os_string();
    temp.push(format!(".shed-{}", runner::pid()));
    if p.mode.is_some_and(|m| m > 0o7777) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "mode over 0o7777",
        ));
    }
    let old = fs::metadata(&p.path).ok().map(|m| m.permissions().mode());
    let mode = p.mode.or(old);
    let write = || {
        let mut file = fs::OpenOptions::new();
        file.write(true).create(true).truncate(true);
        let mut file = file.mode(mode.unwrap_or(0o666)).open(&temp)?;
        if let Some(mode) = mode {
            file.set_permissions(fs::Permissions::from_mode(mode))?;
        }
        file.write_all(&data)
    };
    write()
        .and_then(|()| fs::rename(&temp, &p.path))
        .inspect_err(|_| _ = fs::remove_file(&temp))?;
    Ok(empty())
}

fn lines(file: fs::File, span: Span) -> io::Result<Vec<u8>> {
    let mut reader = BufReader::new(file);
    let (mut bytes, mut line) = (Vec::new(), Vec::new());
    for n in 1..=span.end {
        if reader.read_until(b'\n', &mut line)? == 0 {
            break;
        }
        if n >= span.start {
            bytes.append(&mut line);
            within_limit(bytes.len())?;
        }
        line.clear();
    }
    Ok(bytes)
}

pub(crate) fn within_limit(len: usize) -> io::Result<()> {
    if u64::try_from(len).is_ok_and(|len| len <= proto::REPLY_LIMIT) {
        return Ok(());
    }
    let why = format!(
        "{len} bytes, over the reply limit of {}",
        proto::REPLY_LIMIT
    );
    Err(io::Error::new(io::ErrorKind::FileTooLarge, why))
}
