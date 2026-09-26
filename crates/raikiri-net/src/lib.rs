//! raikiri-net — NetworkProvider / ReplacedResolver base implementations.
//!
//! `FileNetworkProvider` (local-file fetch, for tests/examples) and
//! `ImageResolver<N>` (resource loading plus raster decode/cache, implementing
//! both `ReplacedResolver` and `ImagePixelSource` over any `NetworkProvider`).
//!
//! `ssrf_guard` is the non-overridable IP-address safety floor (private /
//! loopback / link-local / CGNAT / metadata ranges). [`UreqHttpProvider`]
//! applies that floor at connect time and on every redirect hop.
//! [`SystemHttpProvider`] is the explicit trusted-network alternative for
//! browser clients that must reach loopback or private addresses.

#[cfg(feature = "http-ureq")] // cov:ignore: attribute line has no executable code
mod deadline_transport;
mod file_provider;
#[cfg(feature = "http-ureq")] // cov:ignore: attribute line has no executable code
mod host_resolver;
#[cfg(feature = "http-ureq")] // cov:ignore: attribute line has no executable code
mod http_provider;
#[cfg(feature = "http-ureq")] // cov:ignore: attribute line has no executable code
mod http_resolver;
mod image_decoder; // cov:ignore: module declaration has no executable line
mod image_resolver;
mod resource_loader; // cov:ignore: module declaration has no executable line
pub mod ssrf_guard;

pub use file_provider::FileNetworkProvider;
#[cfg(feature = "http-ureq")] // cov:ignore: attribute line has no executable code
pub use host_resolver::HostResolverOverrides;
#[cfg(feature = "http-ureq")] // cov:ignore: attribute line has no executable code
pub use http_provider::{SystemHttpProvider, UreqHttpProvider};
pub use image_resolver::ImageResolver;
