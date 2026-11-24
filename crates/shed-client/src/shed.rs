//! A server: its socket, a pool of idle connections and a call timeout.

use std::io::BufReader;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use serde_json::Value;
use shed_proto::{Event, EventsParams, Hello, Method, ScreenUpdate, Tool, WatchParams};

use crate::{Calls, Conn, Error, Stream};

const IDLE: usize = 4;

/// A server: its socket, at most four idle connections, a call timeout (10 s by default).
#[derive(Clone)]
pub struct Shed {
    socket: PathBuf,
    idle: Arc<Mutex<Vec<Conn>>>,
    timeout: Option<Duration>,
    tool: Option<Tool>,
}

impl Shed {
    /// The server on `socket`; nothing is dialled until the first call.
    pub fn at(socket: impl Into<PathBuf>) -> Self {
        Self {
            socket: socket.into(),
            idle: Arc::default(),
            timeout: Some(Duration::from_secs(10)),
            tool: None,
        }
    }

    /// Each answer must come within `call` (none: wait for ever); a waiting call adds its own wait.
    #[must_use]
    pub fn timeout(self, call: Option<Duration>) -> Self {
        Self {
            timeout: call,
            ..self
        }
    }

    /// Each connection names the caller's own `tool` at hello, as [`Conn::connect_as`].
    #[must_use]
    pub fn tool(self, tool: Tool) -> Self {
        let tool = Some(tool);
        Self { tool, ..self }
    }

    /// The server's hello.
    ///
    /// # Errors
    /// As [`Conn::connect`].
    pub fn hello(&self) -> Result<Hello, Error> {
        let conn = self.take()?;
        let hello = conn.server().clone();
        self.put(conn);
        Ok(hello)
    }

    /// A connection of the caller's own, outside the pool.
    ///
    /// # Errors
    /// As [`Conn::connect`].
    pub fn connect(&self) -> Result<Conn, Error> {
        match self.tool.clone() {
            Some(tool) => Conn::connect_as(&self.socket, tool),
            None => Conn::connect(&self.socket),
        }
    }

    /// The events of the terminals carrying every one of `params.labels`, after `params.since`.
    ///
    /// # Errors
    /// As [`Conn::stream`].
    pub fn events(&self, params: &EventsParams) -> Result<Stream<Event>, Error> {
        self.connect()?.stream(Method::Events, params)
    }

    /// Terminal `name`'s screen updates, after `since`.
    ///
    /// # Errors
    /// As [`Conn::stream`].
    pub fn watch(&self, name: &str, since: Option<u64>) -> Result<Stream<ScreenUpdate>, Error> {
        let terminal = name.to_owned();
        let params = WatchParams { terminal, since };
        self.connect()?.stream(Method::ScreenWatch, &params)
    }

    /// Attaches to terminal `name`, as [`Conn::attach`].
    ///
    /// # Errors
    /// As [`Conn::attach`].
    pub fn attach(&self, name: &str) -> Result<(BufReader<UnixStream>, UnixStream), Error> {
        self.connect()?.attach(name)
    }

    fn idle(&self) -> MutexGuard<'_, Vec<Conn>> {
        self.idle.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn take(&self) -> Result<Conn, Error> {
        loop {
            let Some(mut conn) = self.idle().pop() else {
                return self.connect();
            };
            if conn.alive() {
                return Ok(conn);
            }
        }
    }

    fn put(&self, conn: Conn) {
        let mut idle = self.idle();
        if idle.len() < IDLE {
            idle.push(conn);
        }
    }

    fn deadline(&self, method: Method, params: &Value) -> Option<Duration> {
        let own = |key: &str| params.get(key).and_then(Value::as_f64);
        let extra = match method {
            Method::Wait => own("timeout")?,
            Method::Exec => own("timeout").unwrap_or(0.0),
            Method::Send => own("draft_wait").unwrap_or(60.0),
            Method::Handover => 10.0,
            _ => 0.0,
        };
        Some(self.timeout? + Duration::try_from_secs_f64(extra).ok()?)
    }
}

impl Calls for Shed {
    fn call_raw(&self, method: Method, params: Value) -> Result<Value, Error> {
        let conn = self.take()?;
        conn.deadline(self.deadline(method, &params))?;
        let answer = conn.exchange(method, params);
        if matches!(answer, Ok(_) | Err(Error::Rpc(_))) {
            self.put(conn);
        }
        answer
    }
}
