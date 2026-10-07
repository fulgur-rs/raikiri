use super::*;

#[test]
fn closed_unused_connection_is_not_recorded_as_a_root_request() {
    let server = TestServer::start(HashMap::from([
        ("/live.html", ("text/plain", b"live".to_vec())),
        ("/", ("text/plain", b"root".to_vec())),
    ]));
    let address = (
        server.base_url.host_str().unwrap(),
        server.base_url.port().unwrap(),
    );
    let mut unused = TcpStream::connect(address).unwrap();
    unused.shutdown(std::net::Shutdown::Write).unwrap();
    let mut response = Vec::new();
    unused.read_to_end(&mut response).unwrap();
    for path in ["/live.html", "/"] {
        let mut stream = TcpStream::connect(address).unwrap();
        write!(
            stream,
            "GET {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n"
        )
        .unwrap();
        let mut response = Vec::new();
        stream.read_to_end(&mut response).unwrap();
        assert!(response.starts_with(b"HTTP/1.1 200 OK"));
    }
    assert_eq!(server.finish(), ["/live.html", "/"]);
}
