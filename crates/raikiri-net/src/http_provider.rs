//! Real HTTP(S) `NetworkProvider`, backed by `ureq` and hardened against
//! SSRF: every address `ureq` would connect to is resolved through this
//! crate's internal `SsrfSafeResolver`, which filters candidates through
//! [`crate::ssrf_guard`] before a connection is ever attempted, and
//! [`UreqHttpProvider::new`] disables proxy pickup so a proxy environment
//! variable cannot silently reroute connections around that filtering.

use std::time::Duration;

use raikiri_traits::{
    Body, FetchedResource, Method as RaikiriMethod, NetworkError, NetworkProvider, PolicyViolation,
    Request, ResourceKind, ViolationType,
};
use ureq::ResponseExt as _;
use url::Url;

use crate::http_resolver::{SsrfBlocked, SsrfSafeResolver};

/// End-to-end time limit for a single `NetworkProvider::fetch` call on
/// `UreqHttpProvider`, from DNS lookup to the end of the response body
/// (redirects included). Sized for one sub-resource (image, stylesheet,
/// font).
const FETCH_TIMEOUT: Duration = Duration::from_secs(30);

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
    /// crate-internal `SsrfSafeResolver`, never uses a proxy, and bounds
    /// every fetch (DNS lookup through the end of the response body,
    /// including redirects) by a 30-second end-to-end timeout.
    pub fn new() -> Self {
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
        let config = ureq::config::Config::builder()
            .proxy(None)
            .timeout_global(Some(FETCH_TIMEOUT))
            .build();
        Self::with_agent(ureq::Agent::with_parts(
            config,
            ureq::unversioned::transport::DefaultConnector::default(),
            SsrfSafeResolver::default(),
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
        && inner.downcast_ref::<SsrfBlocked>().is_some()
    {
        return NetworkError::PolicyViolation(PolicyViolation {
            kind,
            url: url.clone(),
            violation_type: ViolationType::PrivateNetworkBlocked,
            details: "resolved IP is not globally routable".to_owned(),
        });
    }
    match err {
        ureq::Error::StatusCode(code) => NetworkError::Http(code),
        ureq::Error::Io(io_err) => NetworkError::Io(io_err),
        other => NetworkError::Other(other.to_string()),
    }
}

impl NetworkProvider for UreqHttpProvider {
    fn fetch(&self, request: Request) -> Result<FetchedResource, NetworkError> {
        if !matches!(request.url.scheme(), "http" | "https") {
            return Err(NetworkError::Other(format!(
                "UreqHttpProvider only supports http/https URLs, got: {}",
                request.url
            )));
        }

        let kind = request.kind;
        let url_str = request.url.as_str();

        let result = match request.method {
            RaikiriMethod::Get => {
                let mut builder = self.agent.get(url_str);
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
                    // `raikiri_traits::Body` is `#[non_exhaustive]`, so this
                    // arm exists purely to keep the match exhaustive against
                    // future variants added upstream.
                    _ => {
                        return Err(NetworkError::Other(
                            "UreqHttpProvider does not support this Body variant".into(),
                        ));
                    }
                };
                let mut builder = self.agent.post(url_str);
                for (name, value) in &request.headers {
                    builder = builder.header(name.as_str(), value.as_str());
                }
                if let Some(content_type) = request.content_type.as_deref() {
                    builder = builder.header("Content-Type", content_type);
                }
                builder.send(&bytes[..])
            }
            _ => {
                return Err(NetworkError::Other(
                    "UreqHttpProvider only supports GET and POST".into(),
                ));
            }
        };

        let mut response = result.map_err(|e| map_ureq_error(&request.url, kind, e))?;

        let final_url = Url::parse(&response.get_uri().to_string())
            .map_err(|e| NetworkError::Other(format!("invalid final URL: {e}")))?;
        let content_type = response.body().mime_type().map(str::to_owned);
        let encoding = response.body().charset().map(str::to_owned);
        let bytes = response
            .body_mut()
            .read_to_vec()
            .map_err(|e| map_ureq_error(&request.url, kind, e))?;

        Ok(FetchedResource {
            bytes: bytes.into(),
            content_type,
            final_url,
            encoding,
        })
    }
}

#[cfg(test)]
mod tests;
