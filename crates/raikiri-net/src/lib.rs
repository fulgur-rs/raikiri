//! raikiri-net — NetworkProvider / ReplacedResolver base implementations.
//!
//! `FileNetworkProvider` (local-file fetch, for tests/examples) and
//! `ImageResolver<N>` (resource loading plus raster decode/cache, implementing
//! both `ReplacedResolver` and `ImagePixelSource` over any `NetworkProvider`).
//! `Sandboxed*`/`DenyAllPolicy`/`DefaultSandboxPolicy` wrappers are a
//! separate, not-yet-implemented follow-up (see this crate's manifest
//! `description`).

mod file_provider;
#[cfg(feature = "http-ureq")]
mod http_resolver;
mod image_decoder; // cov:ignore: module declaration has no executable line
mod image_resolver;
mod resource_loader; // cov:ignore: module declaration has no executable line
pub mod ssrf_guard;

pub use file_provider::FileNetworkProvider;
pub use image_resolver::ImageResolver;
