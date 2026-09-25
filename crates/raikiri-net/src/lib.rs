//! raikiri-net — NetworkProvider / ReplacedResolver base implementations.
//!
//! `FileNetworkProvider` (local-file fetch, for tests/examples) and
//! `ImageResolver<N>` (resource loading plus raster decode/cache, implementing
//! both `ReplacedResolver` and `ImagePixelSource` over any `NetworkProvider`).
//!
//! `ssrf_guard` is the non-overridable IP-address safety floor (private /
//! loopback / link-local / CGNAT / metadata ranges). `UreqHttpProvider`
//! (behind the `http-ureq` feature) is the first real HTTP(S)
//! `NetworkProvider`, hardened with that floor at connect time and on every
//! redirect hop.

mod file_provider;
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
pub use http_provider::UreqHttpProvider;
pub use image_resolver::ImageResolver;
