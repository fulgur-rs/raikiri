//! Custom `ureq` DNS resolver that enforces [`crate::ssrf_guard`] on every
//! address it returns — including the re-resolution `ureq` performs for
//! each redirect hop, since a hop is just a fresh request to a new URL.

use std::fmt;

use ureq::Error;
use ureq::config::Config;
use ureq::http::Uri;
use ureq::unversioned::resolver::{DefaultResolver, ResolvedSocketAddrs, Resolver};
use ureq::unversioned::transport::NextTimeout;

use crate::ssrf_guard::is_globally_routable;

/// Marker error `SsrfSafeResolver` returns (wrapped in `Error::Other`) when
/// every candidate address for a lookup failed the floor. Kept distinct
/// from `Error::HostNotFound` (which `ureq` itself uses for a genuine "no
/// such host" DNS failure) so callers can tell an SSRF-floor rejection
/// apart from an ordinary DNS problem — see `super::http_provider`'s error
/// mapping.
#[derive(Debug)]
pub(crate) struct SsrfBlocked;

impl fmt::Display for SsrfBlocked {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "every resolved address failed the SSRF safety floor")
    }
}

impl std::error::Error for SsrfBlocked {}

/// Resolver that delegates real DNS/IP-literal resolution to
/// `DefaultResolver`, then drops any candidate address that fails
/// [`is_globally_routable`]. If no candidate survives, returns
/// `Error::Other(Box::new(SsrfBlocked))`.
#[derive(Debug, Default)]
pub(crate) struct SsrfSafeResolver {
    inner: DefaultResolver,
}

impl Resolver for SsrfSafeResolver {
    fn resolve(
        &self,
        uri: &Uri,
        config: &Config,
        timeout: NextTimeout,
    ) -> Result<ResolvedSocketAddrs, Error> {
        let resolved = self.inner.resolve(uri, config, timeout)?;
        let mut safe = self.empty();
        for addr in resolved.iter() {
            if is_globally_routable(addr.ip()) {
                safe.push(*addr);
            }
        }
        if safe.is_empty() {
            return Err(Error::Other(Box::new(SsrfBlocked)));
        }
        Ok(safe)
    }
}

#[cfg(test)]
mod tests {
    use ureq::Timeout;
    use ureq::unversioned::transport::time::Duration;

    use super::*;

    // `NextTimeout` has no `Default` impl in this `ureq` version (its
    // `after`/`reason` fields use `ureq`-internal `Duration`/`Timeout`
    // types, not `std::time::Duration`) and no public constructor either,
    // so tests build one via its public field literal. The specific
    // `reason` is irrelevant here — `SsrfSafeResolver::resolve` never reads
    // it, only forwards it to the wrapped `DefaultResolver`.
    fn some_timeout() -> NextTimeout {
        NextTimeout {
            after: Duration::from_secs(5),
            reason: Timeout::Resolve,
        }
    }

    #[test]
    fn allows_a_public_ip_literal_host() {
        let resolver = SsrfSafeResolver::default();
        let uri: Uri = "http://1.1.1.1/".parse().unwrap();
        let config = Config::default();
        let result = resolver.resolve(&uri, &config, some_timeout());
        assert!(result.is_ok(), "expected Ok, got {result:?}");
        assert!(!result.unwrap().is_empty());
    }

    #[test]
    fn blocks_a_loopback_ip_literal_host() {
        let resolver = SsrfSafeResolver::default();
        let uri: Uri = "http://127.0.0.1/".parse().unwrap();
        let config = Config::default();
        let err = resolver
            .resolve(&uri, &config, some_timeout())
            .expect_err("loopback must be rejected");
        match err {
            Error::Other(inner) => {
                assert!(inner.downcast_ref::<SsrfBlocked>().is_some());
            }
            other => panic!("expected Error::Other(SsrfBlocked), got {other:?}"),
        }
    }
}
