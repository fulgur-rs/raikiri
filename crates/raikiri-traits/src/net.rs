//! Network provider trait + neutral network types.
//!
//! Match the shape of blitz-traits::NetProvider (Finding #6). Raikiri has
//! a synchronous core, so it returns synchronously rather than using callbacks.
//! Policy is applied by wrapping with `raikiri-net::SandboxedNetProvider`.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use bytes::Bytes;
use url::Url;

use crate::page::FormData;
use crate::policy::{PolicyViolation, ResourceKind};

/// List of HTTP headers. Can represent multiple values for one header name.
///
/// Uses `Vec<(String, String)>` rather than `http::HeaderMap` to keep the shape
/// lightweight. Reconsider if needed for blitz-compat.
pub type HeaderMap = Vec<(String, String)>;

/// Synchronous-return network provider trait implemented by consumers.
///
/// Raikiri has no timer thread (round 5 review #2). Consumers manage timeouts
/// in their own async runtime / thread pool.
///
/// Finding #6 (round 7 outstanding item: settle the byte-enforcement strategy
/// before implementing sandboxed-net-provider-impl).
pub trait NetworkProvider: Send + Sync {
    /// Fetches exactly one HTTP hop: if the response is itself a redirect
    /// (a 3xx status carrying a `Location`), returns it as
    /// [`FetchOutcome::Redirect`] instead of silently following it. This is
    /// the method a policy-enforcing caller (see `raikiri-html`'s
    /// `ResourceNetworkProvider`) drives directly, so it can check the
    /// redirect target — and count hops itself — *before* any request ever
    /// reaches it, not after the fact. A provider with no real redirect
    /// concept (serves from memory, reads a local file, ...) always
    /// returns `FetchOutcome::Body`.
    fn fetch_one_hop(&self, request: Request) -> Result<FetchOutcome, NetworkError>;

    /// Fetches `request` to completion, automatically following up to
    /// `MAX_AUTO_REDIRECT_HOPS` redirects via repeated calls to
    /// [`NetworkProvider::fetch_one_hop`]. Applies no policy between hops —
    /// a caller that must enforce a [`crate::ResourcePolicy`] (host
    /// allow-list, redirect hop count, ...) drives `fetch_one_hop` itself
    /// instead of using this method, since by the time this method returns
    /// every hop has already been requested.
    ///
    /// Each redirect is turned into the next request by
    /// `Request::redirected`: a redirect that leaves the origin
    /// does not carry the credentials, and one that would downgrade `https`
    /// to `http` or replay a body to another origin is refused.
    fn fetch(&self, request: Request) -> Result<FetchedResource, NetworkError> {
        let mut current = request;
        for _ in 0..=MAX_AUTO_REDIRECT_HOPS {
            match self.fetch_one_hop(current.clone())? {
                FetchOutcome::Body(resource) => return Ok(resource),
                FetchOutcome::Redirect { location, status } => {
                    current = current.redirected(location, status)?;
                }
            }
        }
        Err(NetworkError::Other("too many redirects".to_owned()))
    }

    /// Optional upper bound for recursive stylesheet imports.
    ///
    /// A provider that wraps a [`crate::ResourcePolicy`] can return its
    /// `max_import_depth()` here. Consumers use a conservative fallback when
    /// this method returns `None`, so adding this hook does not require
    /// existing providers to change their implementation.
    fn max_import_depth(&self) -> Option<u32> {
        None
    }
}

/// Cap on the redirects [`NetworkProvider::fetch`]'s default implementation
/// follows on its own. Matches the historical default most HTTP clients
/// (including `ureq`) use for automatic redirect-following.
pub const MAX_AUTO_REDIRECT_HOPS: u32 = 10;

/// Outcome of [`NetworkProvider::fetch_one_hop`].
#[non_exhaustive]
#[derive(Debug, Clone)]
pub enum FetchOutcome {
    /// The final response body — this hop was not a redirect.
    Body(FetchedResource),
    /// A redirect response naming where it points, not followed.
    Redirect {
        /// The `Location` the redirect points to, already resolved to an
        /// absolute URL against the request that produced it.
        location: Url,
        /// The redirect's HTTP status code (e.g. 301, 302, 303, 307, 308).
        status: u16,
    },
}

/// Complete fetch request information.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct Request {
    /// Target URL (before redirects).
    pub url: Url,
    /// HTTP method.
    pub method: Method,
    /// Content-Type header (for POST bodies).
    pub content_type: Option<String>,
    /// Additional headers (`Accept`, `Referer`, custom, etc.).
    ///
    /// When a redirect leaves the request's origin, `Authorization`,
    /// `Proxy-Authorization`, `Cookie` and `Cookie2` are not sent on to the
    /// target (see `Request::redirected`). Any other header is, so a caller
    /// that attaches another kind of credential should drive
    /// [`NetworkProvider::fetch_one_hop`] itself and decide per hop.
    pub headers: HeaderMap,
    /// Request body.
    pub body: Body,
    /// Optional AbortSignal supplied by the consumer.
    pub signal: Option<AbortSignal>,
    /// Raikiri addition: purpose of the fetch (blitz uses doc_id; raikiri uses context).
    pub kind: ResourceKind,
}

impl Request {
    /// A `GET` for `url` with no body, headers or abort signal.
    pub fn new(url: Url, kind: ResourceKind) -> Self {
        Self {
            url,
            method: Method::Get,
            content_type: None,
            headers: HeaderMap::new(),
            body: Body::Empty,
            signal: None,
            kind,
        }
    }

    /// Set the HTTP method.
    pub fn method(mut self, method: Method) -> Self {
        self.method = method;
        self
    }

    /// Set the `Content-Type` of the body.
    pub fn content_type(mut self, content_type: Option<String>) -> Self {
        self.content_type = content_type;
        self
    }

    /// Set the additional headers.
    pub fn headers(mut self, headers: HeaderMap) -> Self {
        self.headers = headers;
        self
    }

    /// Set the request body.
    pub fn body(mut self, body: Body) -> Self {
        self.body = body;
        self
    }

    /// Set the abort signal.
    pub fn signal(mut self, signal: Option<AbortSignal>) -> Self {
        self.signal = signal;
        self
    }

    /// The request to send to `location` after a redirect with `status` from
    /// this one, which every loop that follows redirects uses so that none of
    /// them carries more to the next hop than it should.
    ///
    /// It follows the "HTTP-redirect fetch" steps of the Fetch Standard where
    /// they protect what the request holds, and refuses what they leave to the
    /// caller:
    ///
    /// - A `POST` redirected with 301 or 302, and any method but `GET`
    ///   redirected with 303, becomes a `GET` without a body, `Content-Type` or
    ///   other header that describes the body.
    /// - When `location` has another origin (scheme, host or port), the
    ///   credential headers named on [`Request::headers`] are dropped, whatever
    ///   the case of their names. A `GET` sends no body, so one left on it is
    ///   dropped there too.
    /// - A redirect from `https` to `http` is refused.
    /// - A body that a 307 or 308 would replay to another origin is refused;
    ///   the Fetch Standard sends it.
    ///
    /// A URL without a host (`file:`, `data:`) has an opaque origin, so a
    /// redirect to or from one always counts as leaving the origin.
    ///
    /// A redirect that stays on the origin keeps the method, headers and body
    /// (307 and 308) as they were.
    #[doc(hidden)]
    pub fn redirected(&self, location: Url, status: u16) -> Result<Request, NetworkError> {
        if self.url.scheme() == "https" && location.scheme() == "http" {
            return Err(NetworkError::Other(
                "refused a redirect that downgrades https to http".to_owned(),
            ));
        }
        let mut next = self.clone();
        let becomes_get = match status {
            301 | 302 => next.method == Method::Post,
            303 => next.method != Method::Get,
            _ => false,
        };
        if becomes_get {
            next.method = Method::Get;
        }
        let leaves_origin = !same_origin(&self.url, &location);
        if becomes_get || (leaves_origin && next.method == Method::Get) {
            next.body = Body::Empty;
            next.content_type = None;
            next.headers
                .retain(|(name, _)| !is_body_header(name.as_str()));
        }
        if leaves_origin {
            next.headers
                .retain(|(name, _)| !is_credential_header(name.as_str()));
            if !matches!(next.body, Body::Empty) {
                return Err(NetworkError::Other(
                    "refused to replay a request body to another origin".to_owned(),
                ));
            }
        }
        next.url = location;
        Ok(next)
    }
}

/// Whether `a` and `b` share an origin: the same scheme, host and port. A URL
/// without a host has an opaque origin, which no other URL shares.
fn same_origin(a: &Url, b: &Url) -> bool {
    a.has_host()
        && a.scheme() == b.scheme()
        && a.host() == b.host()
        && a.port_or_known_default() == b.port_or_known_default()
}

/// Whether `name` is a header that a redirect to another origin must not
/// carry: the ones that hold a credential for the origin it came from.
fn is_credential_header(name: &str) -> bool {
    ["authorization", "proxy-authorization", "cookie", "cookie2"]
        .iter()
        .any(|credential| name.eq_ignore_ascii_case(credential))
}

/// Whether `name` is a header that describes a request body, which a redirect
/// that drops the body drops too.
fn is_body_header(name: &str) -> bool {
    [
        "content-encoding",
        "content-language",
        "content-length",
        "content-location",
        "content-type",
    ]
    .iter()
    .any(|header| name.eq_ignore_ascii_case(header))
}

/// Successful fetch response.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct FetchedResource {
    /// Raw response-body bytes.
    pub bytes: Bytes,
    /// `Content-Type` header (parsed MIME type).
    pub content_type: Option<String>,
    /// Effective URL after redirects.
    pub final_url: Url,
    /// Explicit character encoding (otherwise inferred from MIME or BOM).
    pub encoding: Option<String>,
}

impl FetchedResource {
    /// A response of `bytes` served from `final_url`, with no declared type or
    /// encoding.
    pub fn new(bytes: Bytes, final_url: Url) -> Self {
        Self {
            bytes,
            content_type: None,
            final_url,
            encoding: None,
        }
    }

    /// Set the `Content-Type` (parsed MIME type).
    pub fn content_type(mut self, content_type: Option<String>) -> Self {
        self.content_type = content_type;
        self
    }

    /// Set the explicit character encoding.
    pub fn encoding(mut self, encoding: Option<String>) -> Self {
        self.encoding = encoding;
        self
    }
}

/// Representation of a request body.
#[non_exhaustive]
#[derive(Debug, Clone)]
pub enum Body {
    /// Raw byte body.
    Bytes(Bytes),
    /// application/x-www-form-urlencoded body.
    Form(FormData),
    /// No body (GET, etc.).
    Empty,
}

/// HTTP method (extensible; round 3 review #3 correction: HTTP methods are
/// extensible by spec, hence `#[non_exhaustive]`).
///
/// Currently only GET / POST (fulgur's primary use case). Add PUT / DELETE /
/// PATCH / HEAD / OPTIONS when implementing sandboxed-net-provider-impl or
/// when consumers need them. `#[non_exhaustive]` prevents accidental breaks
/// to exhaustive matches in consumer code.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Method {
    /// GET.
    Get,
    /// POST.
    Post,
}

/// AbortSignal with the same shape as blitz (AtomicBool wrapper).
///
/// When a consumer calls `AbortController::abort()`, the shared AtomicBool
/// becomes `true`, observable both by raikiri and the consumer.
#[derive(Debug, Clone)]
pub struct AbortSignal(Arc<AtomicBool>);

impl AbortSignal {
    /// Whether the signal has been aborted.
    pub fn is_aborted(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }

    /// Alias compatible with blitz-traits (`is_aborted` → `aborted`).
    ///
    /// Alias matching the shape of blitz `AbortSignal::aborted()`.
    /// `is_aborted` is canonical.
    pub fn aborted(&self) -> bool {
        self.is_aborted()
    }
}

/// Controller producing an AbortSignal. Round 5 review #2: raikiri never
/// spawns a timer thread. Consumers manage timeouts in their own async
/// runtime.
#[derive(Debug, Default)]
#[non_exhaustive]
pub struct AbortController {
    /// Signal managed by this controller.
    pub signal: AbortSignal,
}

impl AbortController {
    /// Create a new controller.
    pub fn new() -> Self {
        Self {
            signal: AbortSignal(Arc::new(AtomicBool::new(false))),
        }
    }

    /// Transition the signal to the aborted state.
    pub fn abort(&self) {
        self.signal.0.store(true, Ordering::Release);
    }
}

impl Default for AbortSignal {
    fn default() -> Self {
        Self(Arc::new(AtomicBool::new(false)))
    }
}

/// Network-layer error. Reproduces the five variants in §4.
#[non_exhaustive]
#[derive(Debug)]
pub enum NetworkError {
    /// Aborted by an AbortSignal.
    Aborted,
    /// ResourcePolicy violation (raised by SandboxedNetProvider).
    ///
    /// `PolicyViolation` contains a `Url` and exceeds 100 bytes, so it is boxed.
    /// Keeping it inline would enlarge the Err side of every `Result` returning
    /// `NetworkError` or a wrapping `ResolverError` / `LayoutError` / `RenderError`.
    PolicyViolation(Box<PolicyViolation>),
    /// I/O error.
    Io(std::io::Error),
    /// HTTP status code error (4xx / 5xx).
    Http(u16),
    /// Other error.
    Other(String),
}

impl std::fmt::Display for NetworkError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Aborted => write!(f, "Network fetch aborted"),
            Self::PolicyViolation(v) => write!(f, "Network fetch violated policy: {v}"),
            Self::Io(e) => write!(f, "Network I/O error: {e}"),
            Self::Http(status) => write!(f, "Network HTTP status error: {status}"),
            Self::Other(msg) => write!(f, "Network error: {msg}"),
        }
    }
}

impl std::error::Error for NetworkError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::PolicyViolation(v) => Some(&**v),
            Self::Io(e) => Some(e),
            Self::Aborted | Self::Http(_) | Self::Other(_) => None,
        }
    }
}

#[cfg(test)]
mod tests;
