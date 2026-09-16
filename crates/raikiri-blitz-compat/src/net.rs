//! blitz_traits::net compatible shape (blitz-traits =0.3.0-beta.2).
//! Re-implements `blitz_traits::net` verbatim without depending on the crate,
//! then bridges to `raikiri_traits::NetworkProvider`.

pub use bytes::Bytes;
pub use http::{self, HeaderMap, Method};
use std::ops::Deref;
use std::path::PathBuf;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
pub use url::Url;

/// Fetch provider — blitz shape: async via handler callback.
pub trait NetProvider: Send + Sync + 'static {
    fn fetch(&self, doc_id: usize, request: Request, handler: Box<dyn NetHandler>);

    /// Whether this provider never delivers resources, so callers must not
    /// register anything it is asked for as "pending critical" — the
    /// completion callback would never fire and painting would block forever.
    ///
    /// Defaulted to `false` so existing implementors keep compiling, matching
    /// the upstream trait.
    fn is_noop(&self) -> bool {
        false
    }
}

/// Parses raw bytes and calls handler.
pub trait NetHandler: Send + Sync + 'static {
    fn bytes(self: Box<Self>, resolved_url: String, bytes: Bytes);
}

pub trait NetWaker: Send + Sync + 'static {
    fn wake(&self, client_id: usize);
}

impl<F: Fn(usize) + Send + Sync + 'static> NetWaker for F {
    fn wake(&self, doc_id: usize) {
        self(doc_id)
    }
}

#[non_exhaustive]
#[derive(Debug, Clone)]
pub struct Request {
    pub url: Url,
    pub method: Method,
    pub content_type: Option<String>,
    pub headers: HeaderMap,
    pub body: Body,
    pub signal: Option<AbortSignal>,
}

impl Request {
    pub fn get(url: Url) -> Self {
        Self {
            url,
            method: Method::GET,
            content_type: None,
            headers: HeaderMap::new(),
            body: Body::Empty,
            signal: None,
        }
    }

    pub fn signal(mut self, signal: AbortSignal) -> Self {
        self.signal = Some(signal);
        self
    }
}

#[derive(Debug, Clone)]
pub enum Body {
    Bytes(Bytes),
    Form(FormData),
    Empty,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct FormData(pub Vec<Entry>);

impl FormData {
    pub fn new() -> Self {
        FormData(Vec::new())
    }
}

impl Deref for FormData {
    type Target = Vec<Entry>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub name: String,
    pub value: EntryValue,
}

#[derive(Debug, Clone, PartialEq)]
pub enum EntryValue {
    String(String),
    File(PathBuf),
    EmptyFile,
}

impl AsRef<str> for EntryValue {
    fn as_ref(&self) -> &str {
        match self {
            EntryValue::String(s) => s,
            EntryValue::File(p) => p.to_str().unwrap_or_default(),
            EntryValue::EmptyFile => "",
        }
    }
}

impl From<&str> for EntryValue {
    fn from(value: &str) -> Self {
        EntryValue::String(value.to_string())
    }
}

impl From<PathBuf> for EntryValue {
    fn from(value: PathBuf) -> Self {
        EntryValue::File(value)
    }
}

#[derive(Default)]
pub struct DummyNetProvider;
impl NetProvider for DummyNetProvider {
    fn fetch(&self, _doc_id: usize, _request: Request, _handler: Box<dyn NetHandler>) {}
    fn is_noop(&self) -> bool {
        true
    }
}

#[derive(Debug, Default)]
pub struct AbortController {
    pub signal: AbortSignal,
}

impl AbortController {
    pub fn abort(self) {
        self.signal.0.store(true, Ordering::SeqCst);
    }

    /// Non-consuming variant matching raikiri-traits ergonomics.
    pub fn abort_ref(&self) {
        self.signal.0.store(true, Ordering::SeqCst);
    }
}

#[derive(Debug, Default, Clone)]
pub struct AbortSignal(pub(crate) Arc<AtomicBool>);

impl AbortSignal {
    pub fn aborted(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }

    /// raikiri-traits compatible alias.
    pub fn is_aborted(&self) -> bool {
        self.aborted()
    }
}

// ── Conversions to/from raikiri_traits ─────────────────────────────────

/// Convert blitz `Request` → raikiri `Request` (kind defaults to `Other`).
pub fn to_raikiri_request(req: Request) -> raikiri_traits::Request {
    let headers = req
        .headers
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or_default().to_string()))
        .collect::<Vec<_>>();
    let body = match req.body {
        Body::Bytes(b) => raikiri_traits::Body::Bytes(b),
        Body::Empty => raikiri_traits::Body::Empty,
        Body::Form(_fd) => {
            // Raikiri FormData is currently an empty placeholder; preserve shape.
            raikiri_traits::Body::Form(raikiri_traits::FormData::default())
        }
    };
    let signal = req.signal.map(|s| {
        // Bridge AtomicBool
        let raikiri_signal = raikiri_traits::AbortSignal::default();
        if s.aborted() {
            // Propagate aborted state
            // We cannot directly set inner bool without API, so create a new signal
            // that is already aborted.
            let ctrl = raikiri_traits::AbortController::new();
            ctrl.abort();
            ctrl.signal
        } else {
            raikiri_signal
        }
    });
    raikiri_traits::Request {
        url: req.url,
        method: match req.method {
            Method::GET => raikiri_traits::Method::Get,
            Method::POST => raikiri_traits::Method::Post,
            _ => raikiri_traits::Method::Get,
        },
        content_type: req.content_type,
        headers,
        body,
        signal,
        kind: raikiri_traits::ResourceKind::Other,
    }
}

/// Convert raikiri `Request` → blitz `Request`.
pub fn from_raikiri_request(req: raikiri_traits::Request) -> Request {
    let mut headers = HeaderMap::new();
    for (k, v) in req.headers {
        if let (Ok(name), Ok(value)) = (
            http::header::HeaderName::from_bytes(k.as_bytes()),
            http::header::HeaderValue::from_str(&v),
        ) {
            headers.insert(name, value);
        }
    }
    let body = match req.body {
        raikiri_traits::Body::Bytes(b) => Body::Bytes(b),
        raikiri_traits::Body::Empty => Body::Empty,
        raikiri_traits::Body::Form(_) => Body::Form(FormData::default()),
        _ => Body::Empty,
    };
    let signal = req
        .signal
        .map(|s| AbortSignal(Arc::new(AtomicBool::new(s.is_aborted()))));
    Request {
        url: req.url,
        method: match req.method {
            raikiri_traits::Method::Get => Method::GET,
            raikiri_traits::Method::Post => Method::POST,
            _ => Method::GET,
        },
        content_type: req.content_type,
        headers,
        body,
        signal,
    }
}

/// Adapter: wraps a `raikiri_traits::NetworkProvider` and exposes it as `blitz_traits::net::NetProvider`.
///
/// Calls `raikiri_provider.fetch()` synchronously and forwards bytes to the blitz handler.
/// `doc_id` is ignored (raikiri request carries `ResourceKind::Other`).
///
/// `is_noop` keeps its `false` default regardless of the wrapped provider:
/// `fetch` below always reaches `handler.bytes(..)`, delivering empty bytes on
/// error, so the completion callback fires unconditionally — which is exactly
/// the property `is_noop` exists to let callers test. There is no
/// `raikiri_traits::NetworkProvider` counterpart to forward anyway.
pub struct RaikiriNetProviderAdapter<P>(pub P);

impl<P> RaikiriNetProviderAdapter<P> {
    pub fn new(provider: P) -> Self {
        Self(provider)
    }
}

impl<P> NetProvider for RaikiriNetProviderAdapter<P>
where
    P: raikiri_traits::NetworkProvider + 'static,
{
    fn fetch(&self, _doc_id: usize, request: Request, handler: Box<dyn NetHandler>) {
        let raikiri_req = to_raikiri_request(request);
        // For the adapter, we synthesize a FetchedResource → handler bytes call.
        // If fetch errors, we deliver empty bytes (blitz handler has no error channel).
        match self.0.fetch(raikiri_req) {
            Ok(res) => {
                let url = res.final_url.to_string();
                handler.bytes(url, res.bytes);
            }
            Err(_) => {
                // Deliver empty on error; consumer can handle via other channel if needed.
                handler.bytes(String::new(), Bytes::new());
            }
        }
    }
}

/// Reverse adapter: wraps a blitz `NetProvider` and exposes it as raikiri `NetworkProvider`.
///
/// Since blitz fetch is callback-based and raikiri expects sync return, this adapter
/// blocks on a one-shot channel (best-effort sync bridge). Suitable for tests / compat.
pub struct BlitzNetProviderAdapter<P>(pub P);

impl<P> BlitzNetProviderAdapter<P> {
    pub fn new(provider: P) -> Self {
        Self(provider)
    }
}

impl<P> raikiri_traits::NetworkProvider for BlitzNetProviderAdapter<P>
where
    P: NetProvider,
{
    fn fetch(
        &self,
        request: raikiri_traits::Request,
    ) -> Result<raikiri_traits::FetchedResource, raikiri_traits::NetworkError> {
        let blitz_req = from_raikiri_request(request);
        let (tx, rx) = std::sync::mpsc::channel::<(String, Bytes)>();
        struct ChannelHandler {
            tx: std::sync::mpsc::Sender<(String, Bytes)>,
        }
        impl NetHandler for ChannelHandler {
            fn bytes(self: Box<Self>, resolved_url: String, bytes: Bytes) {
                let _ = self.tx.send((resolved_url, bytes));
            }
        }
        self.0.fetch(0, blitz_req, Box::new(ChannelHandler { tx }));
        // Block with timeout to avoid hang; use 5s default.
        match rx.recv_timeout(std::time::Duration::from_secs(5)) {
            Ok((url_str, bytes)) => {
                let url = url::Url::parse(&url_str)
                    .unwrap_or_else(|_| url::Url::parse("about:blank").unwrap());
                Ok(raikiri_traits::FetchedResource {
                    bytes,
                    content_type: None,
                    final_url: url,
                    encoding: None,
                })
            }
            Err(_) => Err(raikiri_traits::NetworkError::Other(
                "blitz adapter timeout".to_string(),
            )),
        }
    }
}
