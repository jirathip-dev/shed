//! The machine side of the token proxy: one loopback port, kept for good, whose connections go byte for byte
//! to the socket the main host's newest forward binds; it holds no key and no token.

use std::io;
use std::net::{Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

use tokio::net::{TcpListener, TcpStream, UnixStream};

use crate::keys::random_hex;

/// The relay: its loopback port and the forward's socket path it relays to.
pub struct Relay {
    addr: SocketAddr,
    dir: PathBuf,
    socket: Arc<Mutex<PathBuf>>,
}

async fn pass(mut tcp: TcpStream, socket: PathBuf) -> io::Result<()> {
    let mut up = UnixStream::connect(socket).await?;
    tokio::io::copy_bidirectional(&mut tcp, &mut up).await?;
    Ok(())
}

impl Relay {
    /// Binds 127.0.0.1 on the port kept beside the state file `state` (a new one the first time), its forward
    /// sockets in `dir`, and relays from then on.
    ///
    /// # Errors
    /// When no loopback port can be bound or the port cannot be kept.
    pub async fn start(state: &Path, dir: &Path) -> io::Result<Self> {
        let kept = state.with_file_name("relay-port");
        let port = std::fs::read_to_string(&kept)
            .ok()
            .and_then(|p| p.trim().parse().ok());
        let loopback = |port| SocketAddr::from((Ipv4Addr::LOCALHOST, port));
        let listener = match TcpListener::bind(loopback(port.unwrap_or(0))).await {
            Ok(listener) => listener,
            Err(_) => TcpListener::bind(loopback(0)).await?,
        };
        let addr = listener.local_addr()?;
        std::fs::write(&kept, addr.port().to_string())?;
        let socket = Arc::new(Mutex::new(PathBuf::new()));
        let current = Arc::clone(&socket);
        tokio::spawn(async move {
            while let Ok((tcp, _)) = listener.accept().await {
                let to = current
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .clone();
                tokio::spawn(pass(tcp, to));
            }
        });
        Ok(Self {
            addr,
            dir: dir.to_owned(),
            socket,
        })
    }

    /// The address it listens on.
    #[must_use]
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// A fresh socket path for the next forward, which it relays to from now on.
    ///
    /// # Errors
    /// `IO` when no random bytes can be read.
    pub fn next(&self) -> Result<PathBuf, shed_proto::RpcError> {
        let path = self.dir.join(format!("up-{}.sock", random_hex(4)?));
        let mut socket = self.socket.lock().unwrap_or_else(PoisonError::into_inner);
        path.clone_into(&mut socket);
        Ok(path)
    }
}

#[cfg(test)]
mod tests {
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
    use tokio::net::{TcpStream, UnixListener};

    use super::Relay;
    use crate::store::tests::temp;

    #[tokio::test]
    async fn it_binds_loopback_on_its_kept_port_and_passes_bytes_to_the_newest_forward() {
        let dir = temp("relay");
        let state = dir.path().join("state.json");
        let relay = Relay::start(&state, dir.path()).await.expect("started");
        assert!(relay.addr().ip().is_loopback(), "{}", relay.addr());
        let kept = std::fs::read_to_string(dir.path().join("relay-port")).expect("kept");
        assert_eq!(kept, relay.addr().port().to_string());
        let forward = UnixListener::bind(relay.next().expect("a path")).expect("bound");
        let mut tcp = TcpStream::connect(relay.addr()).await.expect("connected");
        tcp.write_all(b"ping").await.expect("sent");
        let (mut up, _) = forward.accept().await.expect("relayed");
        let mut got = [0; 4];
        up.read_exact(&mut got).await.expect("read");
        assert_eq!(&got, b"ping");
        up.write_all(b"pong").await.expect("answered");
        tcp.read_exact(&mut got).await.expect("read back");
        assert_eq!(&got, b"pong");
    }
}
