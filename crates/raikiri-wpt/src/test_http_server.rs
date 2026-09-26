use std::collections::HashMap;
use std::io::{Read as _, Write as _};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use raikiri::Url;

pub(crate) struct TestResponse {
    status: &'static str,
    content_type: &'static str,
    headers: Vec<(&'static str, String)>,
    body: Vec<u8>,
}

impl TestResponse {
    pub(crate) fn ok(content_type: &'static str, body: impl Into<Vec<u8>>) -> Self {
        Self {
            status: "200 OK",
            content_type,
            headers: Vec::new(),
            body: body.into(),
        }
    }

    pub(crate) fn redirect(location: impl Into<String>) -> Self {
        Self {
            status: "302 Found",
            content_type: "text/plain",
            headers: vec![("Location", location.into())],
            body: Vec::new(),
        }
    }
}

impl From<(&'static str, Vec<u8>)> for TestResponse {
    fn from((content_type, body): (&'static str, Vec<u8>)) -> Self {
        Self::ok(content_type, body)
    }
}

pub(crate) struct TestServer {
    base_url: Url,
    requests: Arc<Mutex<Vec<String>>>,
    stop: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}

impl TestServer {
    pub(crate) fn start<R>(routes: HashMap<&'static str, R>) -> Self
    where
        R: Into<TestResponse>,
    {
        let routes = routes
            .into_iter()
            .map(|(path, response)| (path, response.into()))
            .collect();
        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind test server");
        listener
            .set_nonblocking(true)
            .expect("make test server nonblocking");
        let address = listener.local_addr().expect("test server address");
        let requests = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let thread_requests = Arc::clone(&requests);
        let thread_stop = Arc::clone(&stop);
        let thread = thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(10);
            while !thread_stop.load(Ordering::Relaxed) && Instant::now() < deadline {
                match listener.accept() {
                    Ok((mut stream, _)) => serve_one(&mut stream, &routes, &thread_requests),
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(2));
                    }
                    Err(error) => panic!("accept test request: {error}"),
                }
            }
        });
        Self {
            base_url: Url::parse(&format!("http://{address}/")).expect("test base URL"),
            requests,
            stop,
            thread: Some(thread),
        }
    }

    pub(crate) fn url(&self, path: &str) -> Url {
        self.base_url.join(path).expect("test URL")
    }

    pub(crate) fn finish(mut self) -> Vec<String> {
        self.stop.store(true, Ordering::Relaxed);
        self.thread.take().expect("server thread").join().unwrap();
        self.requests.lock().unwrap().clone()
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            thread.join().unwrap();
        }
    }
}

fn serve_one(
    stream: &mut TcpStream,
    routes: &HashMap<&'static str, TestResponse>,
    requests: &Mutex<Vec<String>>,
) {
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .expect("set request timeout");
    let mut request = [0_u8; 4096];
    let length = stream.read(&mut request).expect("read request");
    let request = String::from_utf8_lossy(&request[..length]);
    let path = request
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .unwrap_or("/")
        .to_owned();
    requests.lock().unwrap().push(path.clone());
    let missing = TestResponse::ok("text/plain", b"missing".to_vec());
    let response = routes.get(path.as_str()).unwrap_or(&missing);
    write!(
        stream,
        "HTTP/1.1 {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n",
        if routes.contains_key(path.as_str()) {
            response.status
        } else {
            "404 Not Found"
        },
        response.content_type,
        response.body.len()
    )
    .expect("write response headers");
    for (name, value) in &response.headers {
        write!(stream, "{name}: {value}\r\n").expect("write response header");
    }
    write!(stream, "\r\n").expect("finish response headers");
    stream
        .write_all(&response.body)
        .expect("write response body");
}
