use super::*;

fn url() -> Url {
    Url::parse("https://example.test/a").unwrap()
}

#[test]
fn a_request_starts_as_a_plain_get_and_takes_each_other_part_from_a_setter() {
    let request = Request::new(url(), ResourceKind::Image);
    assert_eq!(request.url, url());
    assert_eq!(request.method, Method::Get);
    assert_eq!(request.content_type, None);
    assert!(request.headers.is_empty());
    assert!(matches!(request.body, Body::Empty));
    assert!(request.signal.is_none());
    assert!(matches!(request.kind, ResourceKind::Image));

    let controller = AbortController::new();
    let request = request
        .method(Method::Post)
        .content_type(Some("text/plain".to_owned()))
        .headers(vec![("X-Test".to_owned(), "yes".to_owned())])
        .body(Body::Bytes(Bytes::from_static(b"a=1")))
        .signal(Some(controller.signal.clone()));
    assert_eq!(request.method, Method::Post);
    assert_eq!(request.content_type.as_deref(), Some("text/plain"));
    assert_eq!(request.headers, [("X-Test".to_owned(), "yes".to_owned())]);
    assert!(matches!(&request.body, Body::Bytes(bytes) if bytes.as_ref() == b"a=1"));
    controller.abort();
    assert!(request.signal.is_some_and(|signal| signal.is_aborted()));
}

#[test]
fn a_fetched_resource_starts_without_a_type_or_an_encoding() {
    let resource = FetchedResource::new(Bytes::from_static(b"body"), url());
    assert_eq!(resource.bytes.as_ref(), b"body");
    assert_eq!(resource.final_url, url());
    assert_eq!(resource.content_type, None);
    assert_eq!(resource.encoding, None);

    let resource = resource
        .content_type(Some("text/css".to_owned()))
        .encoding(Some("utf-8".to_owned()));
    assert_eq!(resource.content_type.as_deref(), Some("text/css"));
    assert_eq!(resource.encoding.as_deref(), Some("utf-8"));
}

fn post(url: &str) -> Request {
    Request::new(Url::parse(url).unwrap(), ResourceKind::Image)
        .method(Method::Post)
        .content_type(Some("text/plain".to_owned()))
        .headers(vec![
            ("Authorization".to_owned(), "Bearer secret".to_owned()),
            ("Content-Type".to_owned(), "text/plain".to_owned()),
            ("X-Keep".to_owned(), "yes".to_owned()),
        ])
        .body(Body::Bytes(Bytes::from_static(b"a=1")))
}

fn names(request: &Request) -> Vec<&str> {
    request
        .headers
        .iter()
        .map(|(name, _)| name.as_str())
        .collect()
}

fn redirected(request: &Request, location: &str, status: u16) -> Result<Request, NetworkError> {
    request.redirected(Url::parse(location).unwrap(), status)
}

fn refusal(result: Result<Request, NetworkError>) -> String {
    match result {
        Err(NetworkError::Other(message)) => message,
        other => panic!("expected a refusal, got {other:?}"),
    }
}

#[test]
fn a_redirect_within_the_origin_keeps_the_request_as_it_was() {
    let request = post("https://example.test/a");
    // The default port is the same origin as no port.
    for location in ["https://example.test/b", "https://example.test:443/b?q=1"] {
        for status in [307, 308] {
            let next = redirected(&request, location, status).unwrap();
            assert_eq!(next.url.as_str(), Url::parse(location).unwrap().as_str());
            assert_eq!(next.method, Method::Post);
            assert_eq!(next.content_type.as_deref(), Some("text/plain"));
            assert_eq!(names(&next), ["Authorization", "Content-Type", "X-Keep"]);
            assert!(matches!(&next.body, Body::Bytes(bytes) if bytes.as_ref() == b"a=1"));
        }
    }
    // A GET stays a GET whatever the status.
    let get = Request::new(
        Url::parse("https://example.test/a").unwrap(),
        ResourceKind::Font,
    )
    .headers(vec![("Authorization".to_owned(), "x".to_owned())]);
    for status in [301, 302, 303, 307, 308] {
        let next = redirected(&get, "https://example.test/b", status).unwrap();
        assert_eq!(next.method, Method::Get);
        assert_eq!(names(&next), ["Authorization"]);
    }
}

#[test]
fn a_redirect_to_another_origin_drops_the_credentials_in_any_case() {
    let credentials = [
        "AUTHORIZATION",
        "Cookie",
        "cookie2",
        "Proxy-Authorization",
        "authorization",
    ];
    let mut headers: HeaderMap = credentials
        .iter()
        .map(|name| ((*name).to_owned(), "secret".to_owned()))
        .collect();
    headers.push(("Accept".to_owned(), "text/css".to_owned()));
    headers.push(("X-Test".to_owned(), "yes".to_owned()));
    for (from, to) in [
        ("https://example.test/a", "https://other.test/b"),
        ("https://example.test/a", "https://sub.example.test/b"),
        ("https://example.test/a", "https://example.test:8443/b"),
        ("http://example.test/a", "https://example.test/b"),
        ("http://example.test/a", "http://example.test:8080/b"),
        ("http://example.test/a", "http://example.test:81/b"),
    ] {
        let request =
            Request::new(Url::parse(from).unwrap(), ResourceKind::Image).headers(headers.clone());
        let next = redirected(&request, to, 302).unwrap();
        assert_eq!(names(&next), ["Accept", "X-Test"], "{from} to {to}");
        assert_eq!(next.url.as_str(), to);
    }
}

#[test]
fn a_redirect_that_downgrades_https_is_refused() {
    let secure = post("https://example.test/a");
    for location in ["http://example.test/b", "http://other.test/b"] {
        for status in [301, 302, 303, 307, 308] {
            assert_eq!(
                refusal(redirected(&secure, location, status)),
                "refused a redirect that downgrades https to http"
            );
        }
    }
    // Staying on http, and upgrading to https, are not downgrades.
    let plain = Request::new(
        Url::parse("http://example.test/a").unwrap(),
        ResourceKind::Font,
    );
    assert!(redirected(&plain, "http://example.test/b", 302).is_ok());
    assert!(redirected(&plain, "https://example.test/b", 302).is_ok());
}

#[test]
fn a_post_redirected_with_301_302_or_303_becomes_a_get_without_its_body() {
    let mut request = post("https://example.test/a");
    request
        .headers
        .push(("content-length".to_owned(), "3".to_owned()));
    request
        .headers
        .push(("CONTENT-ENCODING".to_owned(), "gzip".to_owned()));
    request
        .headers
        .push(("Transfer-Encoding".to_owned(), "chunked".to_owned()));
    for status in [301, 302, 303] {
        let next = redirected(&request, "https://example.test/b", status).unwrap();
        assert_eq!(next.method, Method::Get, "{status}");
        assert!(matches!(next.body, Body::Empty), "{status}");
        assert_eq!(next.content_type, None, "{status}");
        // The header that describes the body goes with it; the others stay.
        assert_eq!(names(&next), ["Authorization", "X-Keep"], "{status}");
    }
}

#[test]
fn a_body_is_not_replayed_to_another_origin() {
    let request = post("https://example.test/a");
    for status in [307, 308] {
        assert_eq!(
            refusal(redirected(&request, "https://other.test/b", status)),
            "refused to replay a request body to another origin"
        );
    }
    // Nothing to replay: a POST without a body goes on, without credentials.
    let empty = post("https://example.test/a").body(Body::Empty);
    let next = redirected(&empty, "https://other.test/b", 307).unwrap();
    assert_eq!(next.method, Method::Post);
    assert_eq!(names(&next), ["Content-Type", "X-Keep"]);
    // 301, 302 and 303 drop the body, so they have nothing to refuse.
    for status in [301, 302, 303] {
        let next = redirected(&request, "https://other.test/b", status).unwrap();
        assert_eq!(next.method, Method::Get);
        assert_eq!(names(&next), ["X-Keep"]);
    }
}

#[test]
fn a_get_with_a_body_leaves_the_origin_without_it() {
    // A GET sends no body, so there is nothing to refuse; it must not travel on
    // to another origin all the same.
    let get = Request::new(
        Url::parse("https://example.test/a").unwrap(),
        ResourceKind::Image,
    )
    .content_type(Some("text/plain".to_owned()))
    .headers(vec![
        ("Content-Type".to_owned(), "text/plain".to_owned()),
        ("X-Keep".to_owned(), "yes".to_owned()),
    ])
    .body(Body::Bytes(Bytes::from_static(b"a=1")));
    for status in [301, 302, 303, 307, 308] {
        let next = redirected(&get, "https://other.test/b", status).unwrap();
        assert_eq!(next.method, Method::Get, "{status}");
        assert!(matches!(next.body, Body::Empty), "{status}");
        assert_eq!(next.content_type, None, "{status}");
        assert_eq!(names(&next), ["X-Keep"], "{status}");
    }
    // Within the origin the request is left as it was.
    let next = redirected(&get, "https://example.test/b", 307).unwrap();
    assert!(matches!(&next.body, Body::Bytes(bytes) if bytes.as_ref() == b"a=1"));
}

#[test]
fn a_303_makes_a_get_of_every_method_and_a_301_or_302_only_of_a_post() {
    let post = post("https://example.test/a");
    assert_eq!(
        redirected(&post, "https://example.test/b", 303)
            .unwrap()
            .method,
        Method::Get
    );
    for status in [307, 308] {
        assert_eq!(
            redirected(&post, "https://example.test/b", status)
                .unwrap()
                .method,
            Method::Post
        );
    }
}

#[test]
fn a_url_without_a_host_has_an_origin_of_its_own() {
    let headers = vec![
        ("Authorization".to_owned(), "x".to_owned()),
        ("X-Keep".to_owned(), "yes".to_owned()),
    ];
    for (from, to) in [
        ("file:///a.css", "file:///b.css"),
        ("data:text/css,a", "data:text/css,a"),
        ("file:///a.css", "https://example.test/b"),
        ("https://example.test/a", "file:///b.css"),
    ] {
        let request =
            Request::new(Url::parse(from).unwrap(), ResourceKind::Image).headers(headers.clone());
        let next = redirected(&request, to, 302).unwrap();
        assert_eq!(names(&next), ["X-Keep"], "{from} to {to}");
    }
    // A host makes the origin, whatever the scheme.
    let custom = Request::new(
        Url::parse("custom://example.test/a").unwrap(),
        ResourceKind::Image,
    )
    .headers(headers);
    let next = redirected(&custom, "custom://example.test/b", 302).unwrap();
    assert_eq!(names(&next), ["Authorization", "X-Keep"]);
    let next = redirected(&custom, "custom://other.test/b", 302).unwrap();
    assert_eq!(names(&next), ["X-Keep"]);
}

#[test]
fn a_redirected_request_keeps_its_signal_and_its_kind() {
    let controller = AbortController::new();
    let request = Request::new(
        Url::parse("https://example.test/a").unwrap(),
        ResourceKind::Font,
    )
    .signal(Some(controller.signal.clone()));
    let next = redirected(&request, "https://other.test/b", 302).unwrap();
    assert!(matches!(next.kind, ResourceKind::Font));
    controller.abort();
    assert!(next.signal.is_some_and(|signal| signal.is_aborted()));
}

/// Answers each request with the next redirect of `chain` and, once that is
/// used up, with a body, keeping every request it was sent.
struct RedirectChain {
    chain: Vec<(&'static str, u16)>,
    seen: std::sync::Mutex<Vec<Request>>,
}

impl RedirectChain {
    fn new(chain: Vec<(&'static str, u16)>) -> Self {
        Self {
            chain,
            seen: std::sync::Mutex::new(Vec::new()),
        }
    }
}

impl NetworkProvider for RedirectChain {
    fn fetch_one_hop(&self, request: Request) -> Result<FetchOutcome, NetworkError> {
        let mut seen = self.seen.lock().unwrap();
        let index = seen.len();
        let url = request.url.clone();
        seen.push(request);
        Ok(match self.chain.get(index) {
            Some((location, status)) => FetchOutcome::Redirect {
                location: Url::parse(location).unwrap(),
                status: *status,
            },
            None => FetchOutcome::Body(FetchedResource::new(Bytes::from_static(b"done"), url)),
        })
    }
}

#[test]
fn fetch_sends_each_hop_only_what_its_origin_may_have() {
    let provider = RedirectChain::new(vec![
        ("https://example.test/b", 307),
        ("https://other.test/c", 303),
        ("https://other.test/d", 302),
    ]);
    let fetched = provider.fetch(post("https://example.test/a")).unwrap();
    assert_eq!(fetched.final_url.as_str(), "https://other.test/d");
    let seen = provider.seen.lock().unwrap();
    let hops: Vec<_> = seen
        .iter()
        .map(|request| {
            (
                request.url.path().to_owned(),
                request.method.clone(),
                names(request),
                matches!(request.body, Body::Empty),
            )
        })
        .collect();
    assert_eq!(
        hops,
        [
            (
                "/a".to_owned(),
                Method::Post,
                vec!["Authorization", "Content-Type", "X-Keep"],
                false
            ),
            (
                "/b".to_owned(),
                Method::Post,
                vec!["Authorization", "Content-Type", "X-Keep"],
                false
            ),
            ("/c".to_owned(), Method::Get, vec!["X-Keep"], true),
            ("/d".to_owned(), Method::Get, vec!["X-Keep"], true),
        ]
    );
}

#[test]
fn fetch_does_not_request_a_target_it_refuses() {
    for (location, status, message) in [
        (
            "http://example.test/b",
            302,
            "refused a redirect that downgrades https to http",
        ),
        (
            "https://other.test/b",
            308,
            "refused to replay a request body to another origin",
        ),
    ] {
        let provider = RedirectChain::new(vec![(location, status)]);
        let error = provider.fetch(post("https://example.test/a")).unwrap_err();
        assert!(
            matches!(&error, NetworkError::Other(text) if text == message),
            "{error}"
        );
        assert_eq!(provider.seen.lock().unwrap().len(), 1, "{location}");
    }
}
