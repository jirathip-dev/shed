use std::io::{self, Read as _, Write as _};
use std::net::Shutdown;
use std::os::unix::net::UnixStream;
use std::time::Duration;

use rustix::termios::{
    OptionalActions, SpecialCodeIndex, Termios, tcgetattr, tcgetwinsize, tcsetattr,
};
use shed_server::proto::{Method, ResizeParams, SendParams, Target, TerminalInfo};
use shed_server::{Client, notification};

const DETACH: u8 = 0x1c;
const RESIZE_POLL: Duration = Duration::from_millis(100);

pub(crate) struct RawMode(Option<Termios>);

impl RawMode {
    pub(crate) fn enter(vmin: u8, vtime: u8) -> io::Result<Self> {
        let Ok(saved) = tcgetattr(io::stdin()) else {
            return Ok(Self(None));
        };
        let mut raw = saved.clone();
        raw.make_raw();
        raw.special_codes[SpecialCodeIndex::VMIN] = vmin;
        raw.special_codes[SpecialCodeIndex::VTIME] = vtime;
        tcsetattr(io::stdin(), OptionalActions::Now, &raw)?;
        Ok(Self(Some(saved)))
    }
}

impl Drop for RawMode {
    fn drop(&mut self) {
        if let Some(saved) = &self.0 {
            tcsetattr(io::stdin(), OptionalActions::Now, saved).ok();
        }
    }
}

pub(crate) fn attach(mut client: Client, name: &str) -> anyhow::Result<()> {
    let target = Target {
        name: name.to_owned(),
    };
    let _: TerminalInfo = client.call(Method::Attach, &target)?;
    let (mut reader, writer) = client.split();
    let raw = RawMode::enter(1, 0)?;
    let (resizes, input) = (writer.try_clone()?, writer);
    let owned = name.to_owned();
    std::thread::spawn(move || forward_resizes(&resizes, &owned));
    let owned = name.to_owned();
    std::thread::spawn(move || forward_input(&input, &owned));
    let mut out = io::stdout().lock();
    let mut buf = [0u8; 8192];
    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        out.write_all(buf.get(..n).unwrap_or_default())?;
        out.flush()?;
    }
    drop(raw);
    out.write_all(b"\r\n")?;
    Ok(())
}

fn forward_input(mut conn: &UnixStream, name: &str) {
    let mut stdin = io::stdin().lock();
    let mut buf = [0u8; 1024];
    while let Ok(n) = stdin.read(&mut buf) {
        let read = buf.get(..n).unwrap_or_default();
        let detach = read.iter().position(|&b| b == DETACH);
        let typed = read.get(..detach.unwrap_or(n)).unwrap_or_default();
        if !typed.is_empty() {
            let params = SendParams {
                name: name.to_owned(),
                text: String::from_utf8_lossy(typed).into_owned(),
                ..SendParams::default()
            };
            let sent = notification(Method::Input, &params).map(|line| conn.write_all(&line));
            if !matches!(sent, Ok(Ok(()))) {
                break;
            }
        }
        if detach.is_some() || n == 0 {
            break;
        }
    }
    conn.shutdown(Shutdown::Both).ok();
}

fn forward_resizes(mut conn: &UnixStream, name: &str) {
    let mut last = None;
    loop {
        if let Ok(size) = tcgetwinsize(io::stdin())
            && size.ws_row > 0
            && size.ws_col > 0
            && last != Some((size.ws_row, size.ws_col))
        {
            last = Some((size.ws_row, size.ws_col));
            let params = ResizeParams {
                name: name.to_owned(),
                rows: size.ws_row,
                cols: size.ws_col,
            };
            let sent = notification(Method::Resize, &params).map(|line| conn.write_all(&line));
            if !matches!(sent, Ok(Ok(()))) {
                return;
            }
        }
        shed_server::clock::sleep(RESIZE_POLL);
    }
}
