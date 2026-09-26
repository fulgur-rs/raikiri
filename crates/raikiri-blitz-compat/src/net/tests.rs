use super::*;
use bytes::Bytes;

fn example_url() -> Url {
    Url::parse("https://example.com/page.html").unwrap()
}

#[test]
fn net_waker_blanket_impl_calls_closure() {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };
    let called = Arc::new(AtomicBool::new(false));
    let clone = called.clone();
    let waker = move |doc_id: usize| {
        assert_eq!(doc_id, 7);
        clone.store(true, Ordering::SeqCst);
    };
    NetWaker::wake(&waker, 7);
    assert!(called.load(Ordering::SeqCst));
}

#[test]
fn request_get_defaults_and_signal() {
    let req = Request::get(example_url());
    assert_eq!(req.method, Method::GET);
    assert_eq!(req.content_type, None);
    assert!(req.headers.is_empty());
    assert!(matches!(req.body, Body::Empty));
    assert!(req.signal.is_none());
    let signal = AbortSignal::default();
    let req = req.signal(signal);
    assert!(req.signal.is_some());
}

#[test]
fn form_data_new_default_and_deref() {
    let form = FormData::new();
    assert!(form.is_empty());
    let default = FormData::default();
    assert_eq!(form, default);
    let mut form = FormData::new();
    form.0.push(Entry {
        name: "field".to_string(),
        value: EntryValue::String("value".to_string()),
    });
    assert_eq!(form.len(), 1);
    assert_eq!(form[0].name, "field");
}

#[test]
fn entry_value_from_and_as_ref() {
    let s: EntryValue = "hello".into();
    assert_eq!(s.as_ref() as &str, "hello");
    assert_eq!(EntryValue::EmptyFile.as_ref() as &str, "");
    let path = PathBuf::from("/tmp/file.txt");
    let file: EntryValue = path.clone().into();
    assert_eq!(file.as_ref() as &str, "/tmp/file.txt");
    assert!(matches!(file, EntryValue::File(_)));
}

#[test]
fn dummy_net_provider_fetch_does_not_panic() {
    struct Noop;
    impl NetHandler for Noop {
        fn bytes(self: Box<Self>, _resolved_url: String, _bytes: Bytes) {}
    }
    let provider = DummyNetProvider;
    provider.fetch(0, Request::get(example_url()), Box::new(Noop));
}

#[test]
fn abort_controller_abort_and_ref() {
    let controller = AbortController::default();
    assert!(!controller.signal.aborted());
    assert!(!controller.signal.is_aborted());
    controller.abort_ref();
    assert!(controller.signal.aborted());
    assert!(controller.signal.is_aborted());

    let controller = AbortController::default();
    controller.abort();
    // `abort` consumes; signal state was set before move (covered by no-panic).
}

#[test]
fn to_raikiri_request_maps_methods_and_bodies() {
    let mut req = Request::get(example_url());
    req.method = Method::POST;
    req.body = Body::Bytes(Bytes::from_static(b"data"));
    let out = to_raikiri_request(req);
    assert!(matches!(out.method, raikiri_traits::Method::Post));
    assert!(matches!(out.body, raikiri_traits::Body::Bytes(_)));

    let mut req = Request::get(example_url());
    req.method = Method::PUT;
    req.body = Body::Form(FormData::new());
    let out = to_raikiri_request(req);
    assert!(matches!(out.method, raikiri_traits::Method::Get));
    assert!(matches!(out.body, raikiri_traits::Body::Form(_)));
}

#[test]
fn to_raikiri_request_headers_and_aborted_signal() {
    let mut req = Request::get(example_url());
    req.headers.insert(
        http::header::CONTENT_TYPE,
        http::header::HeaderValue::from_static("text/html"),
    );
    let ctrl = AbortController::default();
    ctrl.abort_ref();
    req.signal = Some(ctrl.signal);
    let out = to_raikiri_request(req);
    assert_eq!(out.headers.len(), 1);
    assert_eq!(out.headers[0].0, "content-type");
    assert_eq!(out.headers[0].1, "text/html");
    let signal = out.signal.expect("signal propagates");
    assert!(signal.is_aborted());
}

#[test]
fn to_raikiri_request_unaborted_signal_stays_clear() {
    let req = Request::get(example_url()).signal(AbortSignal::default());
    let out = to_raikiri_request(req);
    let signal = out.signal.expect("signal propagates");
    assert!(!signal.is_aborted());
}

#[test]
fn from_raikiri_request_maps_methods_bodies_and_signals() {
    let req = raikiri_traits::Request {
        url: example_url(),
        method: raikiri_traits::Method::Post,
        content_type: Some("text/html".to_string()),
        headers: vec![("x-test".to_string(), "1".to_string())],
        body: raikiri_traits::Body::Bytes(Bytes::from_static(b"hi")),
        signal: None,
        kind: raikiri_traits::ResourceKind::Other,
    };
    let out = from_raikiri_request(req);
    assert_eq!(out.method, Method::POST);
    assert!(matches!(out.body, Body::Bytes(_)));
    assert_eq!(
        out.headers
            .get("x-test")
            .map(|v| v.to_str().unwrap().to_string()),
        Some("1".to_string())
    );

    let req = raikiri_traits::Request {
        url: example_url(),
        method: raikiri_traits::Method::Get,
        content_type: None,
        headers: vec![
            ("not a header name \u{0}".to_string(), "1".to_string()),
            ("x-ok".to_string(), "v".to_string()),
        ],
        body: raikiri_traits::Body::Form(raikiri_traits::FormData::default()),
        signal: Some({
            let c = raikiri_traits::AbortController::new();
            c.abort();
            c.signal
        }),
        kind: raikiri_traits::ResourceKind::Other,
    };
    let out = from_raikiri_request(req);
    assert_eq!(out.method, Method::GET);
    assert!(matches!(out.body, Body::Form(_)));
    assert!(out.headers.get("x-ok").is_some());
    assert!(out.signal.expect("signal propagates").aborted());
}

#[test]
fn raikiri_adapter_error_path_delivers_empty_bytes() {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };
    struct Failing;
    impl raikiri_traits::NetworkProvider for Failing {
        fn fetch_one_hop(
            &self,
            _request: raikiri_traits::Request,
        ) -> Result<raikiri_traits::FetchOutcome, raikiri_traits::NetworkError> {
            Err(raikiri_traits::NetworkError::Other("boom".to_string()))
        }
    }
    let provider = RaikiriNetProviderAdapter::new(Failing);
    let called = Arc::new(AtomicBool::new(false));
    let clone = called.clone();
    struct Handler {
        called: Arc<AtomicBool>,
    }
    impl NetHandler for Handler {
        fn bytes(self: Box<Self>, resolved_url: String, bytes: Bytes) {
            assert_eq!(resolved_url, "");
            assert!(bytes.is_empty());
            self.called.store(true, Ordering::SeqCst);
        }
    }
    provider.fetch(
        0,
        Request::get(example_url()),
        Box::new(Handler { called: clone }),
    );
    assert!(called.load(Ordering::SeqCst));
}

#[test]
fn blitz_adapter_success_and_bad_url_fallback() {
    use raikiri_traits::NetworkProvider as _;
    struct Echo {
        url: String,
        body: Bytes,
    }
    impl NetProvider for Echo {
        fn fetch(&self, _doc_id: usize, _request: Request, handler: Box<dyn NetHandler>) {
            handler.bytes(self.url.clone(), self.body.clone());
        }
    }
    let provider = BlitzNetProviderAdapter::new(Echo {
        url: "https://example.com/final".to_string(),
        body: Bytes::from_static(b"ok"),
    });
    let req = raikiri_traits::Request {
        url: example_url(),
        method: raikiri_traits::Method::Get,
        content_type: None,
        headers: vec![],
        body: raikiri_traits::Body::Empty,
        signal: None,
        kind: raikiri_traits::ResourceKind::Other,
    };
    let res = provider.fetch(req).expect("echo succeeds");
    assert_eq!(res.bytes, Bytes::from_static(b"ok"));
    assert_eq!(res.final_url.as_str(), "https://example.com/final");

    let provider = BlitzNetProviderAdapter::new(Echo {
        url: "::: not a url :::".to_string(),
        body: Bytes::from_static(b"x"),
    });
    let req = raikiri_traits::Request {
        url: example_url(),
        method: raikiri_traits::Method::Get,
        content_type: None,
        headers: vec![],
        body: raikiri_traits::Body::Empty,
        signal: None,
        kind: raikiri_traits::ResourceKind::Other,
    };
    let res = provider.fetch(req).expect("bad url falls back");
    assert_eq!(res.final_url.as_str(), "about:blank");
}

#[test]
fn blitz_adapter_timeout_reports_other_error() {
    use raikiri_traits::NetworkProvider as _;
    struct Silent;
    impl NetProvider for Silent {
        fn fetch(&self, _doc_id: usize, _request: Request, _handler: Box<dyn NetHandler>) {
            // Never calls the handler, forcing the 5s timeout arm.
        }
    }
    let provider = BlitzNetProviderAdapter::new(Silent);
    let req = raikiri_traits::Request {
        url: example_url(),
        method: raikiri_traits::Method::Get,
        content_type: None,
        headers: vec![],
        body: raikiri_traits::Body::Empty,
        signal: None,
        kind: raikiri_traits::ResourceKind::Other,
    };
    let err = provider
        .fetch(req)
        .expect_err("silent provider must time out");
    assert!(matches!(err, raikiri_traits::NetworkError::Other(_)));
}
