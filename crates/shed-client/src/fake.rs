//! A fake shed server for tests: a real socket in a temp folder, the answers a test gives it, and the calls
//! it saw. It shares no code with the client, so a client bug cannot hide in it.

use std::io::{self, BufRead as _, BufReader, Write as _};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::{fs, thread};

use serde_json::{Value, json};
use shed_proto::{ErrorCode, Hello, Method, Request, Response, RpcError};

#[derive(Default)]
struct Book {
    answers: Vec<(Method, Result<Value, RpcError>)>,
    seen: Vec<(Method, Value)>,
}

/// A fake server on a socket in a temp folder of its own; dropping it stops it and removes the folder.
pub struct Server {
    dir: PathBuf,
    socket: PathBuf,
    book: Arc<Mutex<Book>>,
    stopped: Arc<AtomicBool>,
}

impl Server {
    /// Starts a server that answers `hello` as shed does and every other method `MethodNotFound`.
    ///
    /// # Errors
    /// When the temp folder or the socket cannot be made.
    pub fn start() -> io::Result<Self> {
        let dir = folder()?;
        let socket = dir.join("shed.sock");
        let listener = match UnixListener::bind(&socket) {
            Ok(listener) => listener,
            Err(e) => {
                fs::remove_dir_all(&dir).ok();
                return Err(e);
            }
        };
        let (book, stopped) = (Arc::<Mutex<Book>>::default(), Arc::<AtomicBool>::default());
        let (shared, stop) = (Arc::clone(&book), Arc::clone(&stopped));
        thread::spawn(move || {
            for stream in listener.incoming().map_while(Result::ok) {
                if stop.load(Ordering::SeqCst) {
                    return;
                }
                let book = Arc::clone(&shared);
                thread::spawn(move || serve(stream, &book));
            }
        });
        Ok(Self {
            dir,
            socket,
            book,
            stopped,
        })
    }

    /// The socket to give the client.
    #[must_use]
    pub fn socket(&self) -> &Path {
        &self.socket
    }

    /// Answers `method` with `result` from now on; a streamed method's array is its items, one line each.
    pub fn answer(&self, method: Method, result: Value) {
        self.set(method, Ok(result));
    }

    /// Answers `method` with `error` from now on.
    pub fn fail(&self, method: Method, error: RpcError) {
        self.set(method, Err(error));
    }

    /// Every call so far but `hello`, in order, with its params.
    #[must_use]
    pub fn seen(&self) -> Vec<(Method, Value)> {
        lock(&self.book).seen.clone()
    }

    fn set(&self, method: Method, reply: Result<Value, RpcError>) {
        let answers = &mut lock(&self.book).answers;
        answers.retain(|(m, _)| *m != method);
        answers.push((method, reply));
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::SeqCst);
        UnixStream::connect(&self.socket).ok();
        fs::remove_dir_all(&self.dir).ok();
    }
}

fn folder() -> io::Result<PathBuf> {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    loop {
        let n = NEXT.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!("shed-fake-{n}"));
        match fs::create_dir(&dir) {
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
            made => return made.map(|()| dir),
        }
    }
}

fn lock(book: &Mutex<Book>) -> MutexGuard<'_, Book> {
    book.lock().unwrap_or_else(PoisonError::into_inner)
}

fn serve(stream: UnixStream, book: &Mutex<Book>) -> Option<()> {
    let mut writer = stream.try_clone().ok()?;
    for line in BufReader::new(stream).lines() {
        let request: Request = serde_json::from_str(&line.ok()?).ok()?;
        let method = Method::ALL.into_iter().find(|m| m.name() == request.method);
        let reply = reply(book, method, &request.method, request.params);
        let Some(id) = request.id else { continue };
        let streamed = matches!(
            method,
            Some(Method::Events | Method::ScreenWatch | Method::Attach)
        );
        let (result, items) = match reply {
            Ok(Value::Array(items)) if streamed => (Ok(json!({})), items),
            reply => (reply, Vec::new()),
        };
        let response = match result {
            Ok(value) => Response::ok(id, value),
            Err(error) => Response::err(id, error),
        };
        writeln!(writer, "{}", serde_json::to_string(&response).ok()?).ok()?;
        if streamed {
            for item in items {
                writeln!(writer, "{item}").ok()?;
            }
            return Some(());
        }
    }
    Some(())
}

fn reply(
    book: &Mutex<Book>,
    method: Option<Method>,
    name: &str,
    params: Value,
) -> Result<Value, RpcError> {
    let unknown = || RpcError::new(ErrorCode::MethodNotFound, name);
    let Some(method) = method else {
        return Err(unknown());
    };
    let mut book = lock(book);
    let given = book.answers.iter().find(|(m, _)| *m == method);
    let given = given.map(|(_, reply)| reply.clone());
    if method == Method::Hello {
        let hello = serde_json::from_value(params).unwrap_or_else(|_| Hello::ours());
        return given.unwrap_or_else(|| Ok(json!(Hello::answer(&hello))));
    }
    book.seen.push((method, params));
    given.unwrap_or_else(|| Err(unknown()))
}
