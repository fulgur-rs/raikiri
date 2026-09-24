//! Real HTTP(S) `NetworkProvider`, backed by `ureq` and hardened against
//! SSRF per `docs/superpowers/specs/2026-09-25-ssrf-protection-design.md`.

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
