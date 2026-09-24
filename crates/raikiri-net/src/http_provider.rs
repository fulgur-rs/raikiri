//! Real HTTP(S) `NetworkProvider`, backed by `ureq` and hardened against
//! SSRF: every address `ureq` would connect to is resolved through
//! [`crate::http_resolver::SsrfSafeResolver`], which filters candidates
//! through [`crate::ssrf_guard`] before a connection is ever attempted, and
//! [`UreqHttpProvider::new`] disables proxy pickup so a proxy environment
//! variable cannot silently reroute connections around that filtering.

use raikiri_traits::{
    Body, FetchedResource, Method as RaikiriMethod, NetworkError, NetworkProvider, PolicyViolation,
    Request, ResourceKind, ViolationType,
};
use ureq::ResponseExt as _;
use url::Url;

use crate::http_resolver::{SsrfBlocked, SsrfSafeResolver};

/// Real HTTP(S) `NetworkProvider`. Always routes through [`SsrfSafeResolver`]
/// — there is no constructor that accepts a different resolver, so the SSRF
/// floor cannot be bypassed by a Consumer of this type.
pub struct UreqHttpProvider {
    agent: ureq::Agent,
}

impl Default for UreqHttpProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl UreqHttpProvider {
    /// Builds a provider whose `Agent` always resolves through
    /// [`SsrfSafeResolver`].
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
        let config = ureq::config::Config::builder().proxy(None).build();
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
mod tests {
    use super::*;

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
}
