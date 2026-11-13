//! The client's side of the version handshake, against a fake server.

use std::io::{BufRead as _, BufReader, Write as _};
use std::os::unix::net::UnixListener;

use shed_server::Client;
use shed_server::proto::{BUILD, PROTOCOL};

#[test]
fn a_server_without_a_protocol_is_refused_by_the_client() {
    let dir = std::env::temp_dir().join(format!("shed-client-{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    std::fs::create_dir_all(&dir).expect("temp dir");
    let socket = dir.join("shed.sock");
    let listener = UnixListener::bind(&socket).expect("bind");
    let fake = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept");
        let mut line = String::new();
        BufReader::new(&stream).read_line(&mut line).expect("hello");
        stream
            .write_all(b"{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{\"version\":\"9.9.9\"}}\n")
            .expect("reply");
    });
    let err = Client::connect(&socket).err().expect("refused");
    fake.join().expect("fake server");
    assert_eq!(
        err.to_string(),
        format!(
            "SHED_VERSION: client {BUILD} / server unnamed unknown: protocol {PROTOCOL} vs 0; restart shed serve"
        )
    );
    std::fs::remove_dir_all(dir).expect("clean");
}
