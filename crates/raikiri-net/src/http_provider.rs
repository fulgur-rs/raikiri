//! Real HTTP(S) `NetworkProvider`, backed by `ureq` and hardened against
//! SSRF: every address `ureq` would connect to is resolved through this
//! crate's internal `SsrfSafeResolver`, which filters candidates through
//! [`crate::ssrf_guard`] before a connection is ever attempted, and
//! [`UreqHttpProvider::new`] disables proxy pickup so a proxy environment
//! variable cannot silently reroute connections around that filtering.

use std::io::{self, Read as _};
use std::time::Duration;

use raikiri_traits::{
    Body, FetchOutcome, FetchedResource, Method as RaikiriMethod, NetworkError, NetworkProvider,
    PolicyViolation, Request, ResourceKind, ViolationType,
};
use ureq::ResponseExt as _;
use ureq::unversioned::transport::Connector as _;
use url::Url;

use crate::http_resolver::{SsrfBlocked, SsrfSafeResolver};

/// Time limit for one hop: [`UreqHttpProvider::fetch_one_hop`], from DNS
/// lookup to the end of that hop's response body, not including any
/// redirect it points to. `fetch()`'s own redirect-following loop (see
/// [`raikiri_traits::NetworkProvider::fetch`]) budgets this fresh for each
/// hop, so a chain of hops is bounded by roughly this value times
/// [`raikiri_traits::MAX_AUTO_REDIRECT_HOPS`], not by this value alone — a
/// caller that must bound total wall-clock time across a whole redirect
/// chain should enforce its own outer deadline. Sized for one sub-resource
/// (image, stylesheet, font). Not a hard deadline over `https://`: see the
/// TLS-read gap described on [`UreqHttpProvider`].
const FETCH_TIMEOUT: Duration = Duration::from_secs(30);

/// Time limit for establishing a connection: TCP connect and, for
/// `https://`, the TLS handshake. Bounds only that phase — it does not
/// close the TLS-read gap described on [`UreqHttpProvider`], which also
/// covers reads made after the handshake completes.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// Upper bound on a response body's size *after* content decoding (gzip),
/// which is the size that actually gets materialized in memory. 32 MiB, the
/// same value `raikiri-html` uses as its default per-resource limit
/// (`raikiri_html::resources::DEFAULT_MAX_RESOURCE_BYTES`), so a document
/// rendered through that layer is never cut off here first. Duplicated as a
/// literal rather than imported: this crate sits below `raikiri-html` and
/// must not depend on it.
const MAX_RESPONSE_BYTES: u64 = 32 * 1024 * 1024;

/// Real HTTP(S) `NetworkProvider`. Always resolves through this crate's
/// internal `SsrfSafeResolver`, which rejects any address that
/// [`crate::ssrf_guard::is_globally_routable`] does not accept — there is no
/// constructor that accepts a different resolver, so the SSRF floor cannot
/// be bypassed by a Consumer of this type.
///
/// This is an application-level IP-address safety floor only, not
/// network-level egress isolation. A deploying Consumer is still expected to
/// run it inside network-level isolation (for example a restricted network
/// namespace with default-deny egress) for defense in depth. The floor also
/// has residual limits: for example, IPv6 transition and legacy prefixes
/// other than IPv4-mapped and the NAT64 well-known prefix (unwrapped and
/// re-checked) and the NAT64 local-use prefix (blocked outright) are not
/// explicitly enumerated, so an address embedded in one of those is not
/// unwrapped and re-checked.
///
/// `ureq` 3.4.2's own timeout accounting is correctly based on an absolute
/// deadline, recomputed (correctly shrinking) each time a new logical
/// operation starts — but its rustls transport adapter takes a snapshot of
/// that value once per operation (once before the TLS handshake, once per
/// logical response read) and reuses it unchanged for every raw socket read
/// `rustls`'s own internal retry loop performs to service that one
/// operation, so a server trickling bytes that each arrive inside that
/// frozen window could otherwise hold one operation open far past the
/// configured deadline. This crate's `deadline_transport` module replaces
/// `ureq`'s own TCP transport with one that re-derives the real remaining
/// time from elapsed wall-clock time on every raw read/write instead of
/// trusting that snapshot, closing this gap without patching `ureq` itself.
pub struct UreqHttpProvider {
    agent: ureq::Agent,
}

impl Default for UreqHttpProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl UreqHttpProvider {
    /// Builds a provider whose `Agent` always resolves through the
    /// crate-internal `SsrfSafeResolver`, never uses a proxy, bounds each
    /// connection attempt (TCP connect plus TLS handshake) by a 10-second
    /// connect timeout, and bounds every hop (DNS lookup through the end of
    /// that hop's response body) by a 30-second timeout that holds even
    /// against a server trickling bytes (see [`UreqHttpProvider`]'s doc). A
    /// multi-hop redirect chain budgets this fresh per hop — see
    /// `FETCH_TIMEOUT`'s doc for what that means for the whole chain.
    pub fn new() -> Self {
        let connector = ()
            .chain(crate::deadline_transport::DeadlineTcpConnector::default())
            .chain(ureq::unversioned::transport::RustlsConnector::default());
        Self::with_agent(ureq::Agent::with_parts(
            agent_config(),
            connector,
            SsrfSafeResolver::new(),
        ))
    }

    /// Not `pub`: only this module (including its `#[cfg(test)]`
    /// submodule) may build a provider around an arbitrary `Agent`. Kept
    /// private rather than `pub(crate)` because nothing outside this file
    /// needs it — a `pub(crate)` escape hatch would still let another
    /// module in this crate construct a floor-bypassing provider by
    /// accident.
    fn with_agent(agent: ureq::Agent) -> Self {
        Self { agent }
    }
}

fn map_ureq_error(url: &Url, kind: ResourceKind, err: ureq::Error) -> NetworkError {
    if let ureq::Error::Other(ref inner) = err
        && let Some(blocked) = inner.downcast_ref::<SsrfBlocked>()
    {
        // Report the URL whose lookup was actually rejected: after a
        // redirect that is the redirect target, not the requested `url`.
        // Falls back to `url` only if the rejected URI is somehow not a
        // valid absolute URL.
        let blocked_url = Url::parse(&blocked.uri.to_string()).unwrap_or_else(|_| url.clone());
        return NetworkError::PolicyViolation(Box::new(PolicyViolation {
            kind,
            url: blocked_url,
            violation_type: ViolationType::PrivateNetworkBlocked,
            details: "resolved IP is not globally routable".to_owned(),
        }));
    }
    match err {
        ureq::Error::StatusCode(code) => NetworkError::Http(code),
        ureq::Error::Io(io_err) => NetworkError::Io(io_err),
        ureq::Error::BodyExceedsLimit(limit) => {
            NetworkError::PolicyViolation(Box::new(PolicyViolation {
                kind,
                url: url.clone(),
                // A limit trips mid-stream, before the full size is known, so
                // `actual` is only a lower bound: the body was at least `limit`
                // bytes long.
                violation_type: ViolationType::FetchTooLarge {
                    limit,
                    actual: limit,
                },
                details: format!("response body exceeds the {limit}-byte limit"),
            }))
        }
        timeout @ ureq::Error::Timeout(_) => {
            NetworkError::Io(io::Error::new(io::ErrorKind::TimedOut, timeout))
        }
        other => NetworkError::Other(other.to_string()),
    }
}

/// Reads `body` to the end, content decoding included, and fails with
/// `ureq::Error::BodyExceedsLimit(cap)` if the *decoded* output is longer
/// than `cap` bytes.
///
/// `ureq`'s own `Body::read_to_vec` limit counts bytes read off the wire,
/// underneath the gzip decoder, so a small compressed body can still
/// decompress to an arbitrarily large `Vec` (a decompression bomb). This
/// instead bounds the decoder's output: reading at most `cap + 1` bytes
/// tells an over-long body apart from one that is exactly `cap` bytes long,
/// without ever buffering more than that. The wire side is not separately
/// capped here; for an unencoded body the output cap bounds it too, and
/// compressed input that yields no output only costs time, which the
/// agent's global timeout bounds.
fn read_body_capped(body: &mut ureq::Body, cap: u64) -> Result<Vec<u8>, ureq::Error> {
    let mut bytes = Vec::new();
    body.as_reader()
        .take(cap.saturating_add(1))
        .read_to_end(&mut bytes)
        // Body-read failures come back as `io::Error`s that may wrap a
        // `ureq::Error` (for example a timeout); `From` unwraps those.
        .map_err(ureq::Error::from)?;
    if bytes.len() as u64 > cap {
        return Err(ureq::Error::BodyExceedsLimit(cap));
    }
    Ok(bytes)
}

/// The `Config` every production `UreqHttpProvider` agent is built with
/// (see [`UreqHttpProvider::new`]).
fn agent_config() -> ureq::config::Config {
    // `ureq::config::Config::default()` picks up `HTTP_PROXY` /
    // `HTTPS_PROXY` / `NO_PROXY` (etc.) from the process environment. If
    // a proxy were left configured, `ureq` would resolve and connect to
    // the *proxy's* address rather than the fetch target's, so
    // `SsrfSafeResolver` (which only ever sees what the configured
    // resolver is asked to resolve) would never see the real target's
    // IP at all — the SSRF floor would be silently inert for every
    // request, with no error to signal it. Building through
    // `Config::builder().proxy(None)` instead of `Config::default()`
    // overrides that environment-sniffing default explicitly, so
    // connections are always direct and `SsrfSafeResolver` is always
    // the thing that resolves what gets connected to.
    //
    // `timeout_global` is end-to-end in `ureq` (DNS lookup to the last
    // byte of the response body, across redirects). Without it a slow or
    // trickling server named by attacker-supplied HTML could block
    // `fetch` indefinitely; the provider, not the caller, is responsible
    // for enforcing fetch timeouts.
    //
    // `timeout_connect` additionally bounds TCP connect plus the TLS
    // handshake, so a black-hole or stalled target fails well before the
    // global budget is spent. Both timeouts hold even once bytes are
    // flowing: `Agent::with_parts` below is built with `deadline_transport`'s
    // connector (not `ureq`'s own `DefaultConnector`/`TcpConnector`), which
    // re-derives the real remaining time on every raw socket read/write
    // instead of trusting a single value to stay fresh across however many
    // raw reads one logical `ureq` operation performs — see that module's
    // doc for why `ureq`'s own accounting needs this.
    ureq::config::Config::builder()
        .proxy(None)
        .timeout_global(Some(FETCH_TIMEOUT))
        .timeout_connect(Some(CONNECT_TIMEOUT))
        .build()
}

/// Redirect status codes that carry a `Location` to follow. Excludes 300
/// (Multiple Choices, no single `Location` semantics) and 304 (Not
/// Modified, a caching response with no body and typically no `Location`
/// at all) — neither is something a caller should be asked to "follow".
fn is_followable_redirect(status: u16) -> bool {
    matches!(status, 301 | 302 | 303 | 307 | 308)
}

impl NetworkProvider for UreqHttpProvider {
    fn fetch_one_hop(&self, request: Request) -> Result<FetchOutcome, NetworkError> {
        // Only checked up front: a signal aborted after this point does not
        // interrupt a fetch already in progress.
        if request.signal.as_ref().is_some_and(|s| s.is_aborted()) {
            return Err(NetworkError::Aborted);
        }
        if !matches!(request.url.scheme(), "http" | "https") {
            return Err(NetworkError::Other(format!(
                "UreqHttpProvider only supports http/https URLs, got: {}",
                request.url
            )));
        }

        let kind = request.kind;
        let url_str = request.url.as_str();

        // `max_redirects(0)`: this method fetches exactly one hop and
        // reports a 3xx back to the caller instead of following it, so a
        // policy-enforcing caller can check the redirect target before any
        // request is ever sent to it.
        let result = match request.method {
            RaikiriMethod::Get => {
                let mut builder = self.agent.get(url_str).config().max_redirects(0).build();
                for (name, value) in &request.headers {
                    builder = builder.header(name.as_str(), value.as_str());
                }
                builder.call()
            }
            RaikiriMethod::Post => {
                let bytes = match &request.body {
                    Body::Bytes(bytes) => bytes.to_vec(),
                    Body::Empty => Vec::new(),
                    Body::Form(_) => {
                        return Err(NetworkError::Other(
                            "UreqHttpProvider does not yet support Body::Form".into(),
                        ));
                    }
                    // cov:ignore: `raikiri_traits::Body` is
                    // `#[non_exhaustive]`; this crate cannot construct a
                    // fourth variant to exercise this arm from a test, only
                    // `Bytes`/`Form`/`Empty` exist today.
                    _ => {
                        return Err(NetworkError::Other(
                            "UreqHttpProvider does not support this Body variant".into(),
                        ));
                    }
                };
                let mut builder = self.agent.post(url_str).config().max_redirects(0).build();
                for (name, value) in &request.headers {
                    builder = builder.header(name.as_str(), value.as_str());
                }
                if let Some(content_type) = request.content_type.as_deref() {
                    builder = builder.header("Content-Type", content_type);
                }
                builder.send(&bytes[..])
            }
            // cov:ignore: `raikiri_traits::Method` is `#[non_exhaustive]`;
            // this crate cannot construct a third variant to exercise this
            // arm from a test, only `Get`/`Post` exist today.
            _ => {
                return Err(NetworkError::Other(
                    "UreqHttpProvider only supports GET and POST".into(),
                ));
            }
        };

        let mut response = result.map_err(|e| map_ureq_error(&request.url, kind, e))?;

        let status = response.status().as_u16();
        if is_followable_redirect(status) {
            let location = response
                .headers()
                .get(ureq::http::header::LOCATION)
                .and_then(|v| v.to_str().ok())
                .ok_or_else(|| {
                    NetworkError::Other(format!(
                        "redirect response ({status}) had no usable Location header"
                    ))
                })?;
            let resolved = request.url.join(location).map_err(|e| {
                NetworkError::Other(format!("invalid redirect Location {location:?}: {e}"))
            })?;
            return Ok(FetchOutcome::Redirect {
                location: resolved,
                status,
            });
        }

        let final_url = Url::parse(&response.get_uri().to_string())
            .map_err(|e| NetworkError::Other(format!("invalid final URL: {e}")))?;
        let content_type = response.body().mime_type().map(str::to_owned);
        let encoding = response.body().charset().map(str::to_owned);
        let bytes = read_body_capped(response.body_mut(), MAX_RESPONSE_BYTES)
            .map_err(|e| map_ureq_error(&final_url, kind, e))?;

        Ok(FetchOutcome::Body(FetchedResource {
            bytes: bytes.into(),
            content_type,
            final_url,
            encoding,
        }))
    }
}

#[cfg(test)]
mod tests;
