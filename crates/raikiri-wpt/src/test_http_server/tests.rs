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

#[test]
fn incomplete_request_does_not_invent_a_root_request() {
    let server = TestServer::start(HashMap::from([("/", ("text/plain", b"root".to_vec()))]));
    let mut stream = TcpStream::connect((
        server.base_url.host_str().unwrap(),
        server.base_url.port().unwrap(),
    ))
    .unwrap();
    stream.write_all(b"G").unwrap();
    stream.shutdown(std::net::Shutdown::Write).unwrap();
    let mut response = Vec::new();
    stream.read_to_end(&mut response).unwrap();
    assert!(server.finish().is_empty());
    assert!(response.is_empty());
}

#[test]
fn request_headers_can_arrive_one_byte_at_a_time() {
    struct OneByte<R>(R);
    impl<R: std::io::Read> std::io::Read for OneByte<R> {
        fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
            let limit = buffer.len().min(1);
            self.0.read(&mut buffer[..limit])
        }
    }
    let request = b"GET /live.html HTTP/1.1\r\nHost: localhost\r\n\r\n";
    assert_eq!(
        read_request_target(&mut OneByte(&request[..])).unwrap(),
        Some("/live.html".into())
    );
}

#[test]
fn incomplete_and_oversized_headers_are_not_requests() {
    for bytes in [b"GET /live.html HTTP/1.1\r\n".to_vec(), vec![b'G'; 4097]] {
        assert_eq!(read_request_target(&mut &bytes[..]).unwrap(), None);
    }
}

#[test]
fn malformed_request_lines_are_not_requests() {
    for bytes in [
        &b"GET\r\n\r\n"[..],
        &b"GET / HTTP/1.1 extra\r\n\r\n"[..],
        &b"GET / invalid\r\n\r\n"[..],
    ] {
        assert_eq!(read_request_target(&mut &bytes[..]).unwrap(), None);
    }
}
