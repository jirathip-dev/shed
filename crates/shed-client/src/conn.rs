//! One connection past the handshake, the one error type, and a call over a one-exchange transport.

use std::cell::RefCell;
use std::io::{self, BufRead as _, BufReader, BufWriter, Write as _};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Serialize;
use serde::de::{DeserializeOwned, IgnoredAny};
use serde_json::Value;
use shed_proto::{ErrorCode, Hello, Method, Request, Response, RpcError, Target, Tool};

use crate::stream::Stream;
use crate::{Calls, MIN_PROTOCOL};

/// Why a call failed.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Nothing listens on the socket, or the connection broke.
    #[error("{}", io_text(socket, source))]
    Io {
        /// The socket; empty for a connection the caller made.
        socket: PathBuf,
        /// What failed.
        source: io::Error,
    },
    /// Not shed, a server under [`MIN_PROTOCOL`], a refused hello, or a call this server is too old for.
    #[error("{}", version_text(ours, theirs.as_deref(), refusal.as_ref(), needs.as_deref()))]
    Version {
        /// This client's hello.
        ours: Box<Hello>,
        /// The server's, when it answered with one.
        theirs: Option<Box<Hello>>,
        /// The server's refusal of ours.
        refusal: Option<RpcError>,
        /// The method or field (`run.env`) the call used that this server does not know.
        needs: Option<String>,
    },
    /// The server's own error.
    #[error("{0}")]
    Rpc(RpcError),
    /// An answer that could not be read.
    #[error("bad response from the server: {0}")]
    Bad(String),
}

impl Error {
    /// The server's error code, for an [`Error::Rpc`] whose code shed knows.
    #[must_use]
    pub fn code(&self) -> Option<ErrorCode> {
        let Self::Rpc(e) = self else { return None };
        ErrorCode::ALL.into_iter().find(|c| c.code() == e.code)
    }
}

fn io_text(socket: &Path, source: &io::Error) -> String {
    if socket.as_os_str().is_empty() {
        source.to_string()
    } else {
        format!("{}: {source}", socket.display())
    }
}

fn version_text(
    ours: &Hello,
    theirs: Option<&Hello>,
    refusal: Option<&RpcError>,
    needs: Option<&str>,
) -> String {
    let range = |h: &Hello| format!("{} to {}", h.oldest.unwrap_or(h.protocol), h.protocol);
    let head = format!(
        "SHED_VERSION: this client speaks shed protocol {}",
        range(ours)
    );
    let fix =
        "(shed updates itself from its own releases: `shed self-update`, docs/self-update.md)";
    match (theirs, refusal) {
        (Some(t), _) if t.product != "shed" => format!("{head}; {} is not shed", t.label()),
        (Some(t), _) => {
            let side = if ours.protocol < t.oldest.unwrap_or(t.protocol) {
                "this client"
            } else {
                "shed there"
            };
            let (version, label) = (&t.version, t.label());
            let lacks = needs
                .map(|n| format!("; it does not know {n}"))
                .unwrap_or_default();
            format!(
                "{head}; the server is {version} ({label}) protocol {}{lacks}; update {side} {fix}",
                range(t)
            )
        }
        (None, Some(e)) => {
            let why = e.message.trim_start_matches("SHED_VERSION: ");
            format!("{head}; the server refused it ({why}); update the side it names {fix}")
        }
        (None, None) => format!("{head}; the server sent no hello"),
    }
}

pub(crate) fn io_at(socket: &Path) -> impl Fn(io::Error) -> Error {
    let socket = socket.to_owned();
    move |source| Error::Io {
        socket: socket.clone(),
        source,
    }
}

fn bad(e: impl std::fmt::Display) -> Error {
    Error::Bad(e.to_string())
}

pub(crate) fn encode(params: &impl Serialize) -> Result<Value, Error> {
    serde_json::to_value(params).map_err(bad)
}

pub(crate) fn decode<R: DeserializeOwned>(value: Value) -> Result<R, Error> {
    serde_json::from_value(value).map_err(bad)
}

fn answer(line: &str, id: &Value) -> Result<Value, Error> {
    let response: Response = serde_json::from_str(line).map_err(bad)?;
    if response.id != *id {
        return Err(Error::Bad(format!("id {}", response.id)));
    }
    match (response.result, response.error) {
        (_, Some(e)) => Err(Error::Rpc(e)),
        (Some(result), None) => Ok(result),
        (None, None) => Err(bad("neither result nor error")),
    }
}

fn ours() -> Hello {
    Hello {
        oldest: Some(MIN_PROTOCOL),
        ..Hello::ours()
    }
}

fn too_old(server: &Hello, method: Method, e: Error) -> Error {
    let Error::Rpc(rpc) = &e else { return e };
    let field = rpc.message.strip_prefix("INVALID_PARAMS: unknown field ");
    let needs = match (rpc.code == ErrorCode::MethodNotFound.code(), field) {
        (true, _) => method.name().to_owned(),
        (false, Some(field)) => field.to_owned(),
        (false, None) => return e,
    };
    Error::Version {
        ours: Box::new(ours()),
        theirs: Some(Box::new(server.clone())),
        refusal: None,
        needs: Some(needs),
    }
}

fn shaken(answer: Result<Value, Error>) -> Result<Hello, Error> {
    let ours = Box::new(ours());
    let theirs: Hello = match answer {
        Err(Error::Rpc(e)) if e.code == ErrorCode::Version.code() => {
            let refusal = Some(e);
            return Err(Error::Version {
                ours,
                theirs: None,
                refusal,
                needs: None,
            });
        }
        answer => decode(answer?)?,
    };
    if theirs.compatible() && MIN_PROTOCOL <= theirs.protocol {
        return Ok(theirs);
    }
    let theirs = Some(Box::new(theirs));
    Err(Error::Version {
        ours,
        theirs,
        refusal: None,
        needs: None,
    })
}

/// One connection, past the handshake.
pub struct Conn {
    socket: PathBuf,
    wire: RefCell<Wire>,
    server: Hello,
}

struct Wire {
    reader: BufReader<UnixStream>,
    writer: BufWriter<UnixStream>,
    next_id: u64,
}

impl Conn {
    /// Dials the server on `socket` and shakes hands.
    ///
    /// # Errors
    /// [`Error::Io`] when nothing listens; [`Error::Version`] when the server is not one this client speaks to.
    pub fn connect(socket: &Path) -> Result<Self, Error> {
        let stream = UnixStream::connect(socket).map_err(io_at(socket))?;
        Self::on(socket.to_owned(), stream, None)
    }

    /// As [`Conn::connect`], naming the caller's own `tool`: the one binary besides shed's the `install` calls move.
    ///
    /// # Errors
    /// As [`Conn::connect`].
    pub fn connect_as(socket: &Path, tool: Tool) -> Result<Self, Error> {
        let stream = UnixStream::connect(socket).map_err(io_at(socket))?;
        Self::on(socket.to_owned(), stream, Some(tool))
    }

    /// Shakes hands over `stream`: a socket, or one end of a socketpair to `ssh … shed connect`.
    ///
    /// # Errors
    /// As [`Conn::connect`].
    pub fn over(stream: UnixStream) -> Result<Self, Error> {
        Self::on(PathBuf::new(), stream, None)
    }

    fn on(socket: PathBuf, stream: UnixStream, tool: Option<Tool>) -> Result<Self, Error> {
        let reader = BufReader::new(stream.try_clone().map_err(io_at(&socket))?);
        let writer = BufWriter::new(stream);
        let wire = RefCell::new(Wire {
            reader,
            writer,
            next_id: 1,
        });
        let mut conn = Self {
            socket,
            wire,
            server: ours(),
        };
        let hello = encode(&Hello { tool, ..ours() })?;
        conn.server = shaken(conn.exchange(Method::Hello, hello))?;
        Ok(conn)
    }

    /// The server's answer to `hello`: its version, build and protocol.
    #[must_use]
    pub fn server(&self) -> &Hello {
        &self.server
    }

    /// Calls `method` with `params` and decodes its result.
    ///
    /// # Errors
    /// [`Error::Rpc`] when the server answers with an error; [`Error::Io`] or [`Error::Bad`] when the
    /// exchange fails.
    pub fn call<P: Serialize, R: DeserializeOwned>(
        &mut self,
        method: Method,
        params: &P,
    ) -> Result<R, Error> {
        let params = encode(params)?;
        decode(self.exchange(method, params)?)
    }

    pub(crate) fn exchange(&self, method: Method, params: Value) -> Result<Value, Error> {
        let wire = &mut *self.wire.borrow_mut();
        let request = Request::new(wire.next_id, method, params);
        wire.next_id += 1;
        let mut line = serde_json::to_vec(&request).map_err(bad)?;
        line.push(b'\n');
        let io = io_at(&self.socket);
        let sent = wire
            .writer
            .write_all(&line)
            .and_then(|()| wire.writer.flush());
        sent.map_err(&io)?;
        let mut reply = String::new();
        match wire.reader.read_line(&mut reply) {
            Ok(0) => Err(io(io::ErrorKind::UnexpectedEof.into())),
            Ok(_) => answer(&reply, &request.id.unwrap_or_default())
                .map_err(|e| too_old(&self.server, method, e)),
            Err(e) => Err(io(late(wire.reader.get_ref(), e))),
        }
    }

    pub(crate) fn deadline(&self, after: Option<Duration>) -> Result<(), Error> {
        let wire = self.wire.borrow();
        let socket = wire.reader.get_ref();
        socket.set_read_timeout(after).map_err(io_at(&self.socket))
    }

    pub(crate) fn alive(&mut self) -> bool {
        let reader = &mut self.wire.get_mut().reader;
        if reader.get_ref().set_nonblocking(true).is_err() {
            return false;
        }
        let idle = matches!(reader.fill_buf(), Err(e) if e.kind() == io::ErrorKind::WouldBlock);
        reader.get_ref().set_nonblocking(false).is_ok() && idle
    }

    /// Calls `method` (`events`, `screen.watch`) and reads the stream that follows it.
    ///
    /// # Errors
    /// As [`Conn::call`].
    pub fn stream<P: Serialize, T>(
        mut self,
        method: Method,
        params: &P,
    ) -> Result<Stream<T>, Error> {
        self.call::<_, IgnoredAny>(method, params)?;
        Ok(Stream::new(self.wire.into_inner().reader))
    }

    /// Attaches to terminal `name`: the reader carries its output, the writer takes [`notification`]s.
    ///
    /// # Errors
    /// As [`Conn::call`].
    pub fn attach(mut self, name: &str) -> Result<(BufReader<UnixStream>, UnixStream), Error> {
        let target = Target { name: name.into() };
        self.call::<_, IgnoredAny>(Method::Attach, &target)?;
        Ok(self.split())
    }

    /// The connection's reader (with anything it buffered) and writer.
    #[must_use]
    pub fn split(self) -> (BufReader<UnixStream>, UnixStream) {
        let wire = self.wire.into_inner();
        (wire.reader, wire.writer.into_parts().0)
    }
}

impl Calls for Conn {
    fn call_raw(&self, method: Method, params: Value) -> Result<Value, Error> {
        self.exchange(method, params)
    }
}

fn late(socket: &UnixStream, e: io::Error) -> io::Error {
    match socket.read_timeout() {
        Ok(Some(d))
            if matches!(
                e.kind(),
                io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
            ) =>
        {
            let why = format!("no answer from the server within {d:?}");
            io::Error::new(io::ErrorKind::TimedOut, why)
        }
        _ => e,
    }
}

/// One call on a transport that carries one exchange: `exchange` gets the lines to send and gives back
/// every line that came out.
///
/// # Errors
/// [`Error::Io`] when `exchange` fails, else as [`Conn::call`].
pub fn once<P: Serialize, R: DeserializeOwned>(
    method: Method,
    params: &P,
    exchange: impl FnOnce(&str) -> io::Result<String>,
) -> Result<R, Error> {
    let hello = Request::new(1, Method::Hello, encode(&ours())?);
    let call = Request::new(2, method, encode(params)?);
    let mut lines = String::new();
    for request in [hello, call] {
        lines += &serde_json::to_string(&request).map_err(bad)?;
        lines.push('\n');
    }
    let out = exchange(&lines).map_err(io_at(Path::new("")))?;
    let mut out = out.lines();
    let server = shaken(answer(out.next().unwrap_or_default(), &1.into()))?;
    let answer = answer(out.next().unwrap_or_default(), &2.into());
    decode(answer.map_err(|e| too_old(&server, method, e))?)
}

/// One notification line (a request without an id, so no response) for `method` with `params`.
///
/// # Errors
/// [`Error::Bad`] when `params` cannot be serialised.
pub fn notification<P: Serialize>(method: Method, params: &P) -> Result<Vec<u8>, Error> {
    let name = method.name();
    let mut line = format!(r#"{{"jsonrpc":"2.0","method":"{name}","params":"#).into_bytes();
    serde_json::to_writer(&mut line, params).map_err(bad)?;
    line.extend_from_slice(b"}\n");
    Ok(line)
}
