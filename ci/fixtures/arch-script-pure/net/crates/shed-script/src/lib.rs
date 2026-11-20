pub fn connect() -> bool { std::net::TcpStream::connect("localhost:1").is_ok() }
