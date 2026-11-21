//! A stream (`events`, `screen.watch`) on a connection of its own, as a blocking iterator.

use std::io::{self, BufRead as _, BufReader};
use std::marker::PhantomData;
use std::os::unix::net::UnixStream;
use std::time::Duration;

use serde::de::DeserializeOwned;
use serde_json::Value;

/// What a stream gives: an item, a gap, a silent stretch, or a line this client cannot read.
#[derive(Debug, Clone, PartialEq)]
pub enum Streamed<T> {
    /// One item.
    Item(T),
    /// The items up to this seq are gone.
    Reset(u64),
    /// This client fell behind after this seq; the server closes the stream.
    Lagged(u64),
    /// A tick, or the server's heartbeat, passed with nothing new.
    Quiet,
    /// A line this client cannot read, as it came.
    Skipped(String),
}

/// A stream on a connection of its own; it ends when the server closes it.
pub struct Stream<T> {
    reader: BufReader<UnixStream>,
    line: String,
    since: Option<u64>,
    item: PhantomData<fn() -> T>,
}

impl<T> Stream<T> {
    pub(crate) fn new(reader: BufReader<UnixStream>) -> Self {
        Self {
            reader,
            line: String::new(),
            since: None,
            item: PhantomData,
        }
    }

    /// Gives [`Streamed::Quiet`] after each `every` with nothing new, so the caller can look up.
    #[must_use]
    pub fn tick(self, every: Duration) -> Self {
        self.reader.get_ref().set_read_timeout(Some(every)).ok();
        self
    }

    /// The last seq seen: where to resume.
    #[must_use]
    pub fn since(&self) -> Option<u64> {
        self.since
    }
}

impl<T: DeserializeOwned> Iterator for Stream<T> {
    type Item = Streamed<T>;

    fn next(&mut self) -> Option<Streamed<T>> {
        match self.reader.read_line(&mut self.line) {
            Ok(0) => None,
            Ok(_) => {
                let line = std::mem::take(&mut self.line);
                Some(self.read(line.trim_end()))
            }
            Err(e)
                if matches!(
                    e.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                ) =>
            {
                Some(Streamed::Quiet)
            }
            Err(_) => None,
        }
    }
}

impl<T: DeserializeOwned> Stream<T> {
    fn read(&mut self, line: &str) -> Streamed<T> {
        let seq = |v: &Value| v.get("seq").and_then(Value::as_u64);
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            return Streamed::Skipped(line.to_owned());
        };
        let gap = value.get("params").and_then(seq);
        match (value.get("method").and_then(Value::as_str), gap) {
            (Some("heartbeat"), _) => return Streamed::Quiet,
            (Some("reset"), Some(seq)) => {
                self.since = Some(seq);
                return Streamed::Reset(seq);
            }
            (Some("lagged"), Some(seq)) => return Streamed::Lagged(seq),
            _ => {}
        }
        let at = seq(&value);
        match serde_json::from_value(value) {
            Ok(item) => {
                self.since = at.or(self.since);
                Streamed::Item(item)
            }
            Err(_) => Streamed::Skipped(line.to_owned()),
        }
    }
}
