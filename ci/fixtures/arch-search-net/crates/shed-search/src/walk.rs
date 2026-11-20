pub fn dial() -> std::io::Result<std::net::TcpStream> {
    std::net::TcpStream::connect("localhost:1")
}
