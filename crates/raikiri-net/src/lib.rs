//! raikiri-net — NetworkProvider / ReplacedResolver base implementations.
//!
//! `FileNetworkProvider` (local-file fetch, for tests/examples) and
//! `ImageResolver<N>` (PNG fetch+decode+cache, implements both
//! `ReplacedResolver` and `ImagePixelSource` over any `NetworkProvider`).
//! `Sandboxed*`/`DenyAllPolicy`/`DefaultSandboxPolicy` wrappers are a
//! separate, not-yet-implemented follow-up (see this crate's manifest
//! `description`).

mod file_provider;
mod image_resolver;

pub use file_provider::FileNetworkProvider;
pub use image_resolver::ImageResolver;
