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
        ureq::Error::Other(Box::new(SsrfBlocked)),
    );
    assert!(matches!(
        err,
        NetworkError::PolicyViolation(v)
            if matches!(v.violation_type, ViolationType::PrivateNetworkBlocked)
    ));
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
            return Err(ureq::Error::Other(Box::new(SsrfBlocked)));
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
