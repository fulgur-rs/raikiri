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
pub(crate) struct SsrfBlocked {
    /// The URI whose lookup was rejected. For a redirect hop this is the
    /// redirect target, not the originally requested URL, so diagnostics
    /// can name the address that was actually blocked.
    pub(crate) uri: Uri,
}

impl fmt::Display for SsrfBlocked {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "every resolved address for {} failed the SSRF safety floor",
            self.uri
        )
    }
}

impl std::error::Error for SsrfBlocked {}

/// Resolver that delegates real DNS/IP-literal resolution to
/// `DefaultResolver`, then drops any candidate address that fails
/// [`is_globally_routable`]. If no candidate survives, returns
/// `Error::Other(Box::new(SsrfBlocked { .. }))` naming the rejected URI.
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
            return Err(Error::Other(Box::new(SsrfBlocked { uri: uri.clone() })));
        }
        Ok(safe)
    }
}

#[cfg(test)]
mod tests;
