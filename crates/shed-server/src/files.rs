use std::fs;
use std::io::{self, BufRead as _, BufReader, Read as _};

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
    let mode = fs::metadata(&p.path).map(|m| m.permissions());
    fs::write(&temp, data)
        .and_then(|()| mode.map_or(Ok(()), |mode| fs::set_permissions(&temp, mode)))
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
