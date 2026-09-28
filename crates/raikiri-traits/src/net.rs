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
    /// [`MAX_AUTO_REDIRECT_HOPS`] redirects via repeated calls to
    /// [`NetworkProvider::fetch_one_hop`]. Applies no policy between hops —
    /// a caller that must enforce a [`crate::ResourcePolicy`] (host
    /// allow-list, redirect hop count, ...) drives `fetch_one_hop` itself
    /// instead of using this method, since by the time this method returns
    /// every hop has already been requested.
    fn fetch(&self, request: Request) -> Result<FetchedResource, NetworkError> {
        let mut current = request;
        for _ in 0..=MAX_AUTO_REDIRECT_HOPS {
            match self.fetch_one_hop(current.clone())? {
                FetchOutcome::Body(resource) => return Ok(resource),
                FetchOutcome::Redirect { location, .. } => {
                    current.url = location;
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
pub struct Request {
    /// Target URL (before redirects).
    pub url: Url,
    /// HTTP method.
    pub method: Method,
    /// Content-Type header (for POST bodies).
    pub content_type: Option<String>,
    /// Additional headers (`Accept`, `Referer`, custom, etc.).
    pub headers: HeaderMap,
    /// Request body.
    pub body: Body,
    /// Optional AbortSignal supplied by the consumer.
    pub signal: Option<AbortSignal>,
    /// Raikiri addition: purpose of the fetch (blitz uses doc_id; raikiri uses context).
    pub kind: ResourceKind,
}

/// Successful fetch response.
#[derive(Debug, Clone)]
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
