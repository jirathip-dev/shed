//! Bounded terminal history: the scrollback caps, and the raw output log that outlives the server, of
//! which a restart reads only the tail.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write as _};
use std::os::unix::fs::{DirBuilderExt as _, OpenOptionsExt as _};
use std::path::{Path, PathBuf};

use crate::screen::{Screen, Size};

/// Scrollback lines each terminal keeps by default.
pub const LINES: usize = 5_000;
/// Scrollback bytes each terminal keeps by default.
pub const BYTES: usize = 8 << 20;

/// How much history a terminal keeps, and where its output log goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct History {
    /// Scrollback lines kept.
    pub lines: usize,
    /// Scrollback bytes kept, and the most of the log read back on restore.
    pub bytes: usize,
    /// The raw output log, appended to chunk by chunk and kept under twice `bytes` on disk. When it
    /// exists at spawn, its tail seeds the screen.
    pub log: Option<PathBuf>,
}

impl Default for History {
    fn default() -> Self {
        Self {
            lines: LINES,
            bytes: BYTES,
            log: None,
        }
    }
}

impl History {
    fn max(&self) -> u64 {
        u64::try_from(self.bytes).unwrap_or(u64::MAX)
    }

    /// A screen of `size` within the caps, fed the tail of the log (blank without one).
    ///
    /// # Errors
    /// [`io::Error`] when the log exists but cannot be read.
    pub fn restore(&self, size: Size) -> io::Result<Screen> {
        let mut screen = Screen::bounded(size, self.lines, self.bytes);
        let file = self.log.as_ref().map(File::open);
        match file {
            Some(Ok(file)) => {
                screen.feed(&tail(file, self.max())?);
                screen.feed(b"\x1b[?1049l\r\n");
            }
            Some(Err(e)) if e.kind() != io::ErrorKind::NotFound => return Err(e),
            _ => {}
        }
        screen.take_replies();
        screen.take_report();
        Ok(screen)
    }
}

/// At most the last `max` bytes of `r`, read from the cut on (nothing before it), with an incomplete
/// UTF-8 character at the cut skipped.
///
/// # Errors
/// [`io::Error`] when `r` cannot be read.
pub fn tail(mut r: impl Read + Seek, max: u64) -> io::Result<Vec<u8>> {
    let len = r.seek(SeekFrom::End(0))?;
    let cut = len.saturating_sub(max);
    r.seek(SeekFrom::Start(cut))?;
    let mut bytes = Vec::new();
    r.take(max).read_to_end(&mut bytes)?;
    if cut > 0 {
        let partial = bytes.iter().take(3).take_while(|b| *b & 0xc0 == 0x80);
        bytes.drain(..partial.count());
    }
    Ok(bytes)
}

pub(crate) struct Log {
    file: File,
    path: PathBuf,
    len: u64,
    max: u64,
}

fn create(path: &Path) -> io::Result<File> {
    OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .open(path)
}

impl Log {
    pub(crate) fn open(history: &History) -> io::Result<Option<Self>> {
        let Some(path) = &history.log else {
            return Ok(None);
        };
        if let Some(dir) = path.parent() {
            fs::DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(dir)?;
        }
        let file = create(path)?;
        Ok(Some(Self {
            len: file.metadata()?.len(),
            file,
            path: path.clone(),
            max: history.max(),
        }))
    }

    pub(crate) fn append(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.file.write_all(bytes)?;
        self.len += u64::try_from(bytes.len()).unwrap_or(u64::MAX);
        if self.len > self.max.saturating_mul(2) {
            let kept = tail(File::open(&self.path)?, self.max)?;
            let tmp = self.path.with_extension("log.tmp");
            let _ = fs::remove_file(&tmp);
            create(&tmp)?.write_all(&kept)?;
            fs::rename(&tmp, &self.path)?;
            self.file = create(&self.path)?;
            self.len = u64::try_from(kept.len()).unwrap_or(u64::MAX);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Counting<R>(R, u64);

    impl<R: Read> Read for Counting<R> {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            let n = self.0.read(buf)?;
            self.1 += n as u64;
            Ok(n)
        }
    }

    impl<R: Seek> Seek for Counting<R> {
        fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
            self.0.seek(pos)
        }
    }

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("shed-history-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("temp dir");
        dir
    }

    #[test]
    fn tail_reads_only_the_tail_and_skips_a_cut_character() {
        let dir = temp("tail");
        let path = dir.join("t.log");
        let mut old = vec![b'x'; 64 << 20];
        old.extend("é".as_bytes());
        old.extend(vec![b'y'; BYTES - 1]);
        fs::write(&path, &old).expect("write");
        let mut file = Counting(File::open(&path).expect("open"), 0);
        let bytes = tail(&mut file, BYTES as u64).expect("tail");
        assert_eq!(file.1, BYTES as u64, "reads exactly the 8 MiB tail");
        assert_eq!(bytes, vec![b'y'; BYTES - 1]);
        let whole = tail(io::Cursor::new("é".as_bytes()), 8).expect("tail");
        assert_eq!(whole, "é".as_bytes(), "nothing is skipped without a cut");
        fs::remove_dir_all(dir).expect("clean");
    }

    #[test]
    fn the_log_stays_under_twice_the_cap_and_restores_the_tail() {
        let dir = temp("log");
        let history = History {
            lines: 3,
            bytes: 64,
            log: Some(dir.join("sub").join("t.log")),
        };
        let mut log = Log::open(&history).expect("open").expect("log");
        for i in 0..100 {
            log.append(format!("line{i}\r\n").as_bytes())
                .expect("append");
            assert!(log.len <= 128 && log.len == fs::metadata(&log.path).expect("meta").len());
        }
        let size = Size { rows: 2, cols: 20 };
        let screen = history.restore(size).expect("restore");
        assert_eq!(screen.history(100), "line97\nline98\nline99");
        let none = History::default().restore(size).expect("no log");
        assert_eq!(none.history(100), "");
        fs::remove_dir_all(dir).expect("clean");
    }
}
