//! Hostname overrides for trusted system-network clients.

use std::collections::HashMap;
use std::fmt;
use std::net::{IpAddr, SocketAddr};

use ureq::Error;
use ureq::config::Config;
use ureq::http::Uri;
use ureq::unversioned::resolver::{DefaultResolver, ResolvedSocketAddrs, Resolver};
use ureq::unversioned::transport::NextTimeout;

/// Explicit hostname-to-address mappings used by [`crate::SystemHttpProvider`].
///
/// Hostnames are matched case-insensitively and without a terminal dot. An
/// override supplies only the IP address; the port always comes from the URL.
#[derive(Clone, Debug, Default)]
pub struct HostResolverOverrides {
    addresses: HashMap<String, IpAddr>,
}

impl HostResolverOverrides {
    /// Creates an empty mapping.
    pub fn new() -> Self {
        Self::default()
    }

    /// Stores `address` for `host`, returning the previous value if present.
    pub fn insert(&mut self, host: impl Into<String>, address: IpAddr) -> Option<IpAddr> {
        self.addresses.insert(normalize_host(&host.into()), address)
    }

    /// Returns the address registered for `host`, if any.
    pub fn get(&self, host: &str) -> Option<IpAddr> {
        self.addresses.get(&normalize_host(host)).copied()
    }
}

fn normalize_host(host: &str) -> String {
    host.trim_end_matches('.').to_ascii_lowercase()
}

pub(crate) struct OverrideResolver<R = DefaultResolver> {
    overrides: HostResolverOverrides,
    inner: R,
}

impl OverrideResolver<DefaultResolver> {
    pub(crate) fn new(overrides: HostResolverOverrides) -> Self {
        Self {
            overrides,
            inner: DefaultResolver::default(),
        }
    }
}

impl<R: fmt::Debug> fmt::Debug for OverrideResolver<R> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OverrideResolver")
            .field("overrides", &self.overrides)
            .field("inner", &self.inner)
            .finish()
    }
}

impl<R: Resolver> Resolver for OverrideResolver<R> {
    fn resolve(
        &self,
        uri: &Uri,
        config: &Config,
        timeout: NextTimeout,
    ) -> Result<ResolvedSocketAddrs, Error> {
        let Some(authority) = uri.authority() else {
            return self.inner.resolve(uri, config, timeout);
        };
        let Some(address) = self.overrides.get(authority.host()) else {
            return self.inner.resolve(uri, config, timeout);
        };
        let Some(scheme) = uri.scheme() else {
            return self.inner.resolve(uri, config, timeout);
        };
        let Some(host_and_port) = DefaultResolver::host_and_port(scheme, authority) else {
            return self.inner.resolve(uri, config, timeout);
        };
        let Some((_, port)) = host_and_port.rsplit_once(':') else {
            return self.inner.resolve(uri, config, timeout);
        };
        let port = port.parse().map_err(|_| Error::HostNotFound)?;

        let mut resolved = self.empty();
        resolved.push(SocketAddr::new(address, port));
        Ok(resolved)
    }
}

#[cfg(test)]
mod tests;
