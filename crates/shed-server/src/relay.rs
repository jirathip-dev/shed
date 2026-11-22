use std::io::{self, Read as _, Write as _};
use std::net::Shutdown;
use std::os::unix::net::UnixStream;
use std::path::Path;

use rustix::event::{PollFd, PollFlags, poll};

use crate::Error;

/// `shed connect`: carries the protocol between stdin/stdout and the server on `socket` (for
/// `ssh host shed connect`) until either side closes, or whoever reads stdout is gone.
///
/// # Errors
/// [`Error::Io`] when nothing listens on `socket` or the copy fails.
pub fn relay(socket: &Path) -> Result<(), Error> {
    let mut stream = UnixStream::connect(socket)?;
    let mut up = stream.try_clone()?;
    std::thread::spawn(move || {
        io::copy(&mut io::stdin().lock(), &mut up).ok();
        up.shutdown(Shutdown::Write).ok();
    });
    let gone = stream.try_clone()?;
    std::thread::spawn(move || reader_gone(&gone));
    let mut out = io::stdout().lock();
    let mut buf = [0u8; 8192];
    loop {
        let n = stream.read(&mut buf)?;
        if n == 0 {
            return Ok(());
        }
        out.write_all(buf.get(..n).unwrap_or_default())?;
        out.flush()?;
    }
}

fn reader_gone(socket: &UnixStream) {
    let out = io::stdout();
    let mut fds = [PollFd::new(&out, PollFlags::empty())];
    while poll(&mut fds, None) == Err(rustix::io::Errno::INTR) {}
    let hung = PollFlags::ERR | PollFlags::HUP;
    if fds.iter().any(|fd| fd.revents().intersects(hung)) {
        socket.shutdown(Shutdown::Both).ok();
    }
}
