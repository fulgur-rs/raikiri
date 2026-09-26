use super::*;

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

struct TestServer {
    base_url: Url,
    requests: Arc<Mutex<Vec<String>>>,
    stop: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}

impl TestServer {
    fn start(routes: HashMap<&'static str, (&'static str, Vec<u8>)>) -> Self {
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

    fn url(&self, path: &str) -> Url {
        self.base_url.join(path).expect("test URL")
    }

    fn finish(mut self) -> Vec<String> {
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
    routes: &HashMap<&'static str, (&'static str, Vec<u8>)>,
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
    let (status, content_type, body) = match routes.get(path.as_str()) {
        Some((content_type, body)) => ("200 OK", *content_type, body.as_slice()),
        None => ("404 Not Found", "text/plain", b"missing".as_slice()),
    };
    write!(
        stream,
        "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    )
    .expect("write response headers");
    stream.write_all(body).expect("write response body");
}

fn red_png() -> Vec<u8> {
    let mut output = Vec::new();
    let mut encoder = png::Encoder::new(&mut output, 2, 2);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().expect("PNG header");
    writer
        .write_image_data(&[
            255, 0, 0, 255, 255, 0, 0, 255, 255, 0, 0, 255, 255, 0, 0, 255,
        ])
        .expect("PNG pixels");
    writer.finish().expect("finish PNG");
    output
}

fn contains_red_pixel(image: &RenderedImage) -> bool {
    image
        .rgba
        .chunks_exact(4)
        .any(|pixel| pixel[0] > 200 && pixel[1] < 50 && pixel[2] < 50 && pixel[3] > 200)
}

#[test]
fn renders_a_relative_img_resource_from_the_document_base_url() {
    let server = TestServer::start(HashMap::from([
        (
            "/index.html",
            (
                "text/html",
                b"<style>html,body{margin:0}img{display:block;width:8px;height:8px}</style><img src='red.png'>".to_vec(),
            ),
        ),
        ("/red.png", ("image/png", red_png())),
    ]));
    let provider = SystemHttpProvider::new();

    let image = render_screen_url(&provider, server.url("index.html"), 32, 32)
        .expect("render relative image");
    let requests = server.finish();

    assert!(requests.iter().any(|path| path == "/red.png"));
    assert!(contains_red_pixel(&image));
}

#[test]
fn renders_a_relative_css_background_from_the_document_base_url() {
    let server = TestServer::start(HashMap::from([
        (
            "/index.html",
            (
                "text/html",
                b"<style>html,body{margin:0}.tile{width:8px;height:8px;background-image:url('red.png')}</style><div class='tile'></div>".to_vec(),
            ),
        ),
        ("/red.png", ("image/png", red_png())),
    ]));
    let provider = SystemHttpProvider::new();

    let image = render_screen_url(&provider, server.url("index.html"), 32, 32)
        .expect("render relative background");
    let requests = server.finish();

    assert!(requests.iter().any(|path| path == "/red.png"));
    assert!(contains_red_pixel(&image));
}

#[test]
fn fetches_relative_font_face_sources_from_the_document_base_url() {
    let server = TestServer::start(HashMap::from([
        (
            "/index.html",
            (
                "text/html",
                b"<style>@font-face{font-family:Probe;src:url('probe.ttf')}body{font-family:Probe}</style><body>probe</body>".to_vec(),
            ),
        ),
        ("/probe.ttf", ("font/ttf", b"not-a-real-font".to_vec())),
    ]));
    let provider = SystemHttpProvider::new();

    render_screen_url(&provider, server.url("index.html"), 64, 32)
        .expect("render document with font-face");
    let requests = server.finish();

    assert!(requests.iter().any(|path| path == "/probe.ttf"));
}

#[test]
fn resolves_external_stylesheet_resources_from_the_stylesheet_url() {
    let server = TestServer::start(HashMap::from([
        (
            "/pages/index.html",
            (
                "text/html",
                b"<link rel='stylesheet' href='../styles/sheet.css'><div class='tile'>probe</div>"
                    .to_vec(),
            ),
        ),
        (
            "/styles/sheet.css",
            (
                "text/css",
                b"@font-face{font-family:Probe;src:url('probe.ttf')}.tile{width:8px;height:8px;background-image:url('red.png');font-family:Probe}"
                    .to_vec(),
            ),
        ),
        ("/styles/red.png", ("image/png", red_png())),
        (
            "/styles/probe.ttf",
            ("font/ttf", b"not-a-real-font".to_vec()),
        ),
    ]));
    let provider = SystemHttpProvider::new();

    let image = render_screen_url(&provider, server.url("pages/index.html"), 32, 32)
        .expect("render resources relative to external stylesheet");
    let requests = server.finish();

    assert!(requests.iter().any(|path| path == "/styles/red.png"));
    assert!(requests.iter().any(|path| path == "/styles/probe.ttf"));
    assert!(contains_red_pixel(&image));
}
