use super::*;

use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::thread;

use ureq::unversioned::resolver::{DefaultResolver, ResolvedSocketAddrs, Resolver};
use ureq::unversioned::transport::{DefaultConnector, NextTimeout};

fn test_url() -> Url {
    Url::parse("https://example.test/x.png").unwrap()
}

#[test]
fn floor_rejection_maps_to_private_network_blocked() {
    let err = map_ureq_error(
        &test_url(),
        ResourceKind::Image,
        ureq::Error::Other(Box::new(SsrfBlocked {
            uri: "http://10.0.0.1/x.png".parse().unwrap(),
        })),
    );
    match err {
        NetworkError::PolicyViolation(v) => {
            assert!(matches!(
                v.violation_type,
                ViolationType::PrivateNetworkBlocked
            ));
            // The violation names the URI the resolver rejected, not the
            // `url` passed in for context.
            assert_eq!(v.url.as_str(), "http://10.0.0.1/x.png");
        }
        other => panic!("expected PolicyViolation(PrivateNetworkBlocked), got {other:?}"),
    }
}

#[test]
fn floor_rejection_falls_back_to_the_request_url_when_the_uri_is_not_absolute() {
    let err = map_ureq_error(
        &test_url(),
        ResourceKind::Image,
        ureq::Error::Other(Box::new(SsrfBlocked {
            uri: "/relative-only".parse().unwrap(),
        })),
    );
    assert!(
        matches!(&err, NetworkError::PolicyViolation(v) if v.url == test_url()),
        "expected a PolicyViolation reporting the request URL, got {err:?}"
    );
}

#[test]
fn genuine_host_not_found_is_not_mislabeled_as_a_policy_violation() {
    let err = map_ureq_error(&test_url(), ResourceKind::Image, ureq::Error::HostNotFound);
    assert!(
        !matches!(err, NetworkError::PolicyViolation(_)),
        "an ordinary DNS-resolution failure must not be reported as a \
         PrivateNetworkBlocked policy violation, or logs/telemetry \
         cannot tell a real SSRF-floor hit from a typo'd hostname: \
         got {err:?}"
    );
}

#[test]
fn http_status_error_maps_to_network_error_http() {
    let err = map_ureq_error(
        &test_url(),
        ResourceKind::Image,
        ureq::Error::StatusCode(404),
    );
    assert!(matches!(err, NetworkError::Http(404)));
}

// This test asserts that `proxy()` reports `None` regardless of the
// process environment. It only actively *discriminates* the fix (fails
// without it) when a proxy environment variable ureq reads
// (`ALL_PROXY`/`HTTP_PROXY`/`HTTPS_PROXY`, upper or lower case) happens
// to be set in the process running the test — otherwise
// `Proxy::try_from_env()` itself already returns `None`, so an
// unfixed `Config::default()` would also pass this assertion. Not
// mutated via `std::env` in-process here, since that is racy against
// Rust's default parallel test execution. To independently verify this
// assertion is exercising real behavior rather than an environment
// that already has no proxy set, run with a proxy variable present,
// e.g. `HTTPS_PROXY=http://127.0.0.1:9 cargo test -p raikiri-net
// --features http-ureq new_disables_proxy_pickup`, and confirm it
// still passes; then swap `UreqHttpProvider::new`'s `Config` back to
// `ureq::config::Config::default()` and confirm the same command now
// fails.
#[test]
fn new_disables_proxy_pickup_so_the_ssrf_floor_cannot_be_silently_bypassed() {
    let provider = UreqHttpProvider::new();
    assert!(
        provider.agent.config().proxy().is_none(),
        "UreqHttpProvider::new() must explicitly disable proxy usage; \
         otherwise ureq's environment-sniffing default could silently \
         route connections through a proxy's address instead of the \
         fetch target's, making SsrfSafeResolver never see (and never \
         filter) the real destination"
    );
}

#[test]
fn floor_blocks_a_get_to_a_loopback_ip_literal_through_the_real_agent() {
    // Unlike the unit-level `floor_rejection_maps_to_private_network_blocked`
    // test above (which constructs `Error::Other(SsrfBlocked)` by hand),
    // this drives the full path: `UreqHttpProvider::new()`'s real
    // `Agent` (built with `SsrfSafeResolver`) resolving and attempting
    // to connect to an IP-literal URL. No DNS lookup or successful
    // connection is needed either way: 127.0.0.1 is an IP literal (no
    // DNS), and `SsrfSafeResolver` rejects it before any socket is
    // opened, so this is deterministic and makes no real network I/O.
    let provider = UreqHttpProvider::new();
    let request = Request {
        url: Url::parse("http://127.0.0.1:1/").unwrap(),
        method: RaikiriMethod::Get,
        content_type: None,
        headers: Vec::new(),
        body: Body::Empty,
        signal: None,
        kind: ResourceKind::Image,
    };
    let err = provider
        .fetch(request)
        .expect_err("loopback IP literal must be blocked by the SSRF floor");
    assert!(
        matches!(
            &err,
            NetworkError::PolicyViolation(v)
                if matches!(v.violation_type, ViolationType::PrivateNetworkBlocked)
        ),
        "expected NetworkError::PolicyViolation(PrivateNetworkBlocked), got {err:?}"
    );
}

#[test]
fn rejects_non_http_schemes_before_any_network_activity() {
    let provider = UreqHttpProvider::new();
    let request = Request {
        url: Url::parse("file:///etc/passwd").unwrap(),
        method: RaikiriMethod::Get,
        content_type: None,
        headers: Vec::new(),
        body: Body::Empty,
        signal: None,
        kind: ResourceKind::Image,
    };
    let err = provider
        .fetch(request)
        .expect_err("non-http(s) scheme must be rejected");
    assert!(matches!(err, NetworkError::Other(_)));
}

/// Builds a `Config` the same way `UreqHttpProvider::new()` does:
/// explicitly disabling proxy pickup. Every `Agent` built in this test
/// module goes through this helper rather than `Config::default()`, so
/// none of these tests can be silently short-circuited (skipping the
/// configured resolver entirely) by a `HTTP_PROXY`/`HTTPS_PROXY`/etc.
/// environment variable that happens to be set wherever the suite runs.
fn no_proxy_config() -> ureq::config::Config {
    ureq::config::Config::builder().proxy(None).build()
}

/// Stands in for `SsrfSafeResolver` in this one test: delegates real
/// resolution to `DefaultResolver`, then drops any candidate whose
/// *port* matches `blocked_port`. A hermetic test can't stand up a
/// listener on a genuinely public IP to prove "an allowed first hop can
/// redirect to a blocked second hop" using `ssrf_guard`'s real IP-range
/// content (both hops would be 127.0.0.1, which `ssrf_guard` already
/// blocks outright) — so this substitutes port identity for IP-range
/// classification. It exercises the identical mechanism (resolver
/// invoked again per hop, rejection surfaces as `Error::Other(SsrfBlocked)`)
/// that the real resolver uses for actual IP ranges.
#[derive(Debug)]
struct PortBlockingResolver {
    inner: DefaultResolver,
    blocked_port: u16,
}

impl Resolver for PortBlockingResolver {
    fn resolve(
        &self,
        uri: &ureq::http::Uri,
        config: &ureq::config::Config,
        timeout: NextTimeout,
    ) -> Result<ResolvedSocketAddrs, ureq::Error> {
        let resolved = self.inner.resolve(uri, config, timeout)?;
        let mut safe = self.empty();
        for addr in resolved.iter() {
            if addr.port() != self.blocked_port {
                safe.push(*addr);
            }
        }
        if safe.is_empty() {
            return Err(ureq::Error::Other(Box::new(SsrfBlocked {
                uri: uri.clone(),
            })));
        }
        Ok(safe)
    }
}

/// Reads and discards a raw HTTP/1.1 request line + headers from
/// `stream`, then writes `response` back and returns. Good enough for a
/// one-shot test server — these tests do not need a real HTTP parser.
fn serve_one_response(mut stream: std::net::TcpStream, response: &str) {
    let mut reader = BufReader::new(stream.try_clone().expect("clone stream"));
    let mut line = String::new();
    reader.read_line(&mut line).expect("read request line");
    loop {
        let mut header_line = String::new();
        reader
            .read_line(&mut header_line)
            .expect("read header line");
        if header_line == "\r\n" || header_line.is_empty() {
            break;
        }
    }
    stream
        .write_all(response.as_bytes())
        .expect("write response");
}

#[test]
fn redirect_target_is_revalidated_and_blocked_even_though_the_first_hop_was_allowed() {
    // The simulated SSRF target: would answer 200 OK if ever reached.
    let target_listener = TcpListener::bind("127.0.0.1:0").expect("bind target");
    let target_port = target_listener.local_addr().unwrap().port();
    thread::spawn(move || {
        let (stream, _) = target_listener.accept().expect("accept target conn");
        serve_one_response(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nhi",
        );
    });

    // The allowed first hop: redirects to the target above.
    let source_listener = TcpListener::bind("127.0.0.1:0").expect("bind source");
    let source_port = source_listener.local_addr().unwrap().port();
    thread::spawn(move || {
        let (stream, _) = source_listener.accept().expect("accept source conn");
        let response = format!(
            "HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:{target_port}/\r\n\
             Content-Length: 0\r\nConnection: close\r\n\r\n"
        );
        serve_one_response(stream, &response);
    });

    let agent = ureq::Agent::with_parts(
        no_proxy_config(),
        DefaultConnector::default(),
        PortBlockingResolver {
            inner: DefaultResolver::default(),
            blocked_port: target_port,
        },
    );
    let provider = UreqHttpProvider::with_agent(agent);

    let request = Request {
        url: Url::parse(&format!("http://127.0.0.1:{source_port}/")).unwrap(),
        method: RaikiriMethod::Get,
        content_type: None,
        headers: Vec::new(),
        body: Body::Empty,
        signal: None,
        kind: ResourceKind::Image,
    };
    let err = provider
        .fetch(request)
        .expect_err("redirect to the blocked port must fail the whole fetch");

    assert!(
        matches!(
            &err,
            NetworkError::PolicyViolation(v)
                if matches!(v.violation_type, ViolationType::PrivateNetworkBlocked)
        ),
        "the first hop (source) was allowed, but the redirect target \
         was not re-validated and blocked as expected: got {err:?}"
    );
    // The violation must name the blocked redirect target, not the allowed
    // source URL the fetch started from.
    let NetworkError::PolicyViolation(v) = &err else {
        unreachable!()
    };
    assert_eq!(
        v.url.port(),
        Some(target_port),
        "PolicyViolation.url must name the blocked redirect target \
         (port {target_port}), not the source (port {source_port}): got {}",
        v.url
    );
}

#[test]
fn fetch_reads_back_a_real_successful_response_end_to_end() {
    // Plain success-path test: no redirect, no blocking — just a real
    // 200 OK through a one-shot TCP server, to exercise the response
    // reading (`final_url`/`mime_type`/`charset`/`read_to_vec`) that the
    // redirect-hop test above never reaches (it only ever gets as far
    // as the blocked redirect).
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind listener");
    let port = listener.local_addr().unwrap().port();
    let body = "hello world";
    thread::spawn(move || {
        let (stream, _) = listener.accept().expect("accept conn");
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: {}\r\n\
             Connection: close\r\n\r\n{body}",
            body.len()
        );
        serve_one_response(stream, &response);
    });

    // Unfiltered resolver here — this test is about the success path,
    // not the SSRF floor, so there is nothing to block.
    let agent = ureq::Agent::with_parts(
        no_proxy_config(),
        DefaultConnector::default(),
        DefaultResolver::default(),
    );
    let provider = UreqHttpProvider::with_agent(agent);

    let url = Url::parse(&format!("http://127.0.0.1:{port}/")).unwrap();
    let request = Request {
        url: url.clone(),
        method: RaikiriMethod::Get,
        content_type: None,
        headers: Vec::new(),
        body: Body::Empty,
        signal: None,
        kind: ResourceKind::Image,
    };

    let resource = provider.fetch(request).expect("fetch must succeed");
    assert_eq!(resource.bytes.as_ref(), body.as_bytes());
    assert_eq!(resource.content_type.as_deref(), Some("text/plain"));
    assert_eq!(resource.final_url, url);
}

/// Like `serve_one_response`, but for a response whose body is arbitrary
/// bytes (for example gzip data, which is not valid UTF-8). Write errors
/// are ignored: a client that stops reading at its size cap may close the
/// connection while the server is still writing.
fn serve_one_raw_response(stream: std::net::TcpStream, head: &str, body: &[u8]) {
    let mut reader = BufReader::new(stream.try_clone().expect("clone stream"));
    let mut line = String::new();
    reader.read_line(&mut line).expect("read request line");
    loop {
        let mut header_line = String::new();
        reader
            .read_line(&mut header_line)
            .expect("read header line");
        if header_line == "\r\n" || header_line.is_empty() {
            break;
        }
    }
    let mut stream = stream;
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(body);
}

/// Builds a gzip body that is small on the wire but decompresses to
/// `members` MiB of zeros: one 1 MiB gzip member (about 1 KiB compressed)
/// repeated `members` times. Concatenated members are a valid gzip stream
/// (RFC 1952 §2.2) and `ureq` decodes them with a multi-member decoder.
fn gzip_bomb(members: usize) -> Vec<u8> {
    use flate2::Compression;
    use flate2::write::GzEncoder;

    let mut encoder = GzEncoder::new(Vec::new(), Compression::best());
    encoder
        .write_all(&vec![0u8; 1024 * 1024])
        .expect("compress");
    let member = encoder.finish().expect("finish gzip member");
    member.repeat(members)
}

#[test]
fn gzip_decompression_bomb_is_rejected_instead_of_materialized() {
    // 64 MiB decompressed, roughly 64 KiB on the wire: far below ureq's
    // default 10 MiB wire-byte limit, far above the decompressed cap.
    let body = gzip_bomb(64);
    assert!(
        body.len() < 200 * 1024,
        "fixture must stay small on the wire, got {} bytes",
        body.len()
    );

    let listener = TcpListener::bind("127.0.0.1:0").expect("bind listener");
    let port = listener.local_addr().unwrap().port();
    thread::spawn(move || {
        let (stream, _) = listener.accept().expect("accept conn");
        let head = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\n\
             Content-Encoding: gzip\r\nContent-Length: {}\r\n\
             Connection: close\r\n\r\n",
            body.len()
        );
        serve_one_raw_response(stream, &head, &body);
    });

    let agent = ureq::Agent::with_parts(
        no_proxy_config(),
        DefaultConnector::default(),
        DefaultResolver::default(),
    );
    let provider = UreqHttpProvider::with_agent(agent);
    let request = Request {
        url: Url::parse(&format!("http://127.0.0.1:{port}/bomb")).unwrap(),
        method: RaikiriMethod::Get,
        content_type: None,
        headers: Vec::new(),
        body: Body::Empty,
        signal: None,
        kind: ResourceKind::Image,
    };

    match provider.fetch(request) {
        Ok(resource) => panic!(
            "a gzip body decompressing past the cap must be rejected, but \
             fetch returned Ok with {} decompressed bytes",
            resource.bytes.len()
        ),
        Err(err) => assert!(
            matches!(
                &err,
                NetworkError::PolicyViolation(v)
                    if matches!(v.violation_type, ViolationType::FetchTooLarge { .. })
            ),
            "expected PolicyViolation(FetchTooLarge), got {err:?}"
        ),
    }
}

#[test]
fn read_body_capped_accepts_exactly_cap_bytes_and_rejects_one_more() {
    let mut exact = ureq::Body::builder().data(vec![7u8; 16]);
    assert_eq!(read_body_capped(&mut exact, 16).unwrap(), vec![7u8; 16]);

    let mut over = ureq::Body::builder().data(vec![7u8; 17]);
    let err = read_body_capped(&mut over, 16).expect_err("17 bytes must exceed a 16-byte cap");
    assert!(
        matches!(err, ureq::Error::BodyExceedsLimit(16)),
        "expected BodyExceedsLimit(16), got {err:?}"
    );
}

#[test]
fn body_exceeds_limit_maps_to_fetch_too_large_policy_violation() {
    let err = map_ureq_error(
        &test_url(),
        ResourceKind::Image,
        ureq::Error::BodyExceedsLimit(1024),
    );
    match err {
        NetworkError::PolicyViolation(v) => {
            assert!(matches!(
                v.violation_type,
                ViolationType::FetchTooLarge {
                    limit: 1024,
                    actual: 1024
                }
            ));
            assert_eq!(v.url, test_url());
        }
        other => panic!("expected PolicyViolation(FetchTooLarge), got {other:?}"),
    }
}

#[test]
fn timeout_maps_to_a_typed_timed_out_io_error() {
    let err = map_ureq_error(
        &test_url(),
        ResourceKind::Image,
        ureq::Error::Timeout(ureq::Timeout::Global),
    );
    assert!(
        matches!(&err, NetworkError::Io(e) if e.kind() == std::io::ErrorKind::TimedOut),
        "expected NetworkError::Io(TimedOut), got {err:?}"
    );
}

#[test]
fn a_small_gzip_body_is_still_decoded_end_to_end() {
    // Guards the capped read path against silently dropping content
    // decoding: the provider must still hand back decompressed bytes.
    let body = gzip_bomb(1);
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind listener");
    let port = listener.local_addr().unwrap().port();
    thread::spawn(move || {
        let (stream, _) = listener.accept().expect("accept conn");
        let head = format!(
            "HTTP/1.1 200 OK\r\nContent-Encoding: gzip\r\nContent-Length: {}\r\n\
             Connection: close\r\n\r\n",
            body.len()
        );
        serve_one_raw_response(stream, &head, &body);
    });
    let agent = ureq::Agent::with_parts(
        no_proxy_config(),
        DefaultConnector::default(),
        DefaultResolver::default(),
    );
    let provider = UreqHttpProvider::with_agent(agent);
    let request = Request {
        url: Url::parse(&format!("http://127.0.0.1:{port}/")).unwrap(),
        method: RaikiriMethod::Get,
        content_type: None,
        headers: Vec::new(),
        body: Body::Empty,
        signal: None,
        kind: ResourceKind::Image,
    };
    let resource = provider.fetch(request).expect("fetch must succeed");
    assert_eq!(resource.bytes.len(), 1024 * 1024);
    assert!(resource.bytes.iter().all(|&b| b == 0));
}

#[test]
fn production_config_sets_global_and_connect_timeouts() {
    let timeouts = UreqHttpProvider::new().agent.config().timeouts();
    assert_eq!(timeouts.global, Some(FETCH_TIMEOUT));
    assert_eq!(timeouts.connect, Some(CONNECT_TIMEOUT));
    assert!(CONNECT_TIMEOUT < FETCH_TIMEOUT);
}

#[test]
fn a_stalled_tls_handshake_fails_at_the_connect_timeout_as_a_typed_timeout() {
    // The server accepts the TCP connection but never answers the
    // ClientHello. With the production config this must fail once the
    // connect timeout elapses, well before the global timeout, and surface
    // as a typed `TimedOut` I/O error rather than an opaque string.
    use std::time::Instant;

    let listener = TcpListener::bind("127.0.0.1:0").expect("bind listener");
    let port = listener.local_addr().unwrap().port();
    let (done_tx, done_rx) = std::sync::mpsc::channel::<()>();
    thread::spawn(move || {
        let (_stream, _) = listener.accept().expect("accept conn");
        // Hold the connection open, silently, until the client gives up.
        let _ = done_rx.recv();
    });

    // Production timeouts, but an unfiltered resolver so the loopback
    // listener is reachable at all.
    let agent = ureq::Agent::with_parts(
        agent_config(),
        DefaultConnector::default(),
        DefaultResolver::default(),
    );
    let provider = UreqHttpProvider::with_agent(agent);
    let request = Request {
        url: Url::parse(&format!("https://127.0.0.1:{port}/")).unwrap(),
        method: RaikiriMethod::Get,
        content_type: None,
        headers: Vec::new(),
        body: Body::Empty,
        signal: None,
        kind: ResourceKind::Image,
    };

    let start = Instant::now();
    let err = provider
        .fetch(request)
        .expect_err("a silent TLS server must time out");
    let elapsed = start.elapsed();
    let _ = done_tx.send(());

    assert!(
        matches!(&err, NetworkError::Io(e) if e.kind() == std::io::ErrorKind::TimedOut),
        "expected NetworkError::Io(TimedOut), got {err:?}"
    );
    assert!(
        elapsed >= CONNECT_TIMEOUT && elapsed < FETCH_TIMEOUT,
        "expected the connect timeout ({CONNECT_TIMEOUT:?}) to fire before the \
         global one ({FETCH_TIMEOUT:?}), took {elapsed:?}"
    );
}

#[test]
fn an_already_aborted_signal_returns_aborted_before_any_network_activity() {
    // A loopback URL would otherwise be rejected by the SSRF floor, so
    // getting `Aborted` back (not `PolicyViolation`) proves the signal is
    // checked before resolution even starts.
    let controller = raikiri_traits::AbortController::new();
    controller.abort();
    let request = Request {
        url: Url::parse("http://127.0.0.1:1/").unwrap(),
        method: RaikiriMethod::Get,
        content_type: None,
        headers: Vec::new(),
        body: Body::Empty,
        signal: Some(controller.signal.clone()),
        kind: ResourceKind::Image,
    };
    let err = UreqHttpProvider::new()
        .fetch(request)
        .expect_err("an aborted request must not be fetched");
    assert!(
        matches!(err, NetworkError::Aborted),
        "expected Aborted, got {err:?}"
    );
}

#[test]
fn a_not_yet_aborted_signal_does_not_block_the_fetch() {
    let controller = raikiri_traits::AbortController::new();
    let request = Request {
        url: Url::parse("http://127.0.0.1:1/").unwrap(),
        method: RaikiriMethod::Get,
        content_type: None,
        headers: Vec::new(),
        body: Body::Empty,
        signal: Some(controller.signal.clone()),
        kind: ResourceKind::Image,
    };
    // Proceeds to the SSRF floor, which rejects the loopback target.
    let err = UreqHttpProvider::new().fetch(request).unwrap_err();
    assert!(
        matches!(err, NetworkError::PolicyViolation(_)),
        "expected the fetch to proceed to the floor, got {err:?}"
    );
}
