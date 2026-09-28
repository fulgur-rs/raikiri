//! Replaced-element resolver trait + resolve types.
//!
//! Consumer-provided trait returning intrinsic sizes for replaced elements such as
//! `<img>`, `<object>`, `<embed>`, and `<svg>`. Only sizing is handled; the consumer fetches.

use url::Url;

use crate::net::NetworkError;

/// Consumer-side trait for resolving intrinsic sizes of replaced elements.
///
/// - The consumer validates the URL scheme, host, size, etc. in req.
///   Wrap in `raikiri-net::SandboxedResolver` to apply a centralized policy
///   (Finding #6, §10).
/// - Round 4 review #3: `Err` is always terminal (`RenderError::Resolver`).
///   Return a consumer-side fallback as `Ok(ResolvedIntrinsic { intrinsic, disposition:
///   Fallback { .. } })`; raikiri inspects the disposition and automatically
///   records it in `RenderSummary.warnings`.
pub trait ReplacedResolver {
    /// Resolve one replaced element.
    fn resolve(&self, req: ResolverRequest<'_>) -> Result<ResolvedIntrinsic, ResolverError>;
}

/// Intrinsic size.
#[derive(Debug, Default, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub struct IntrinsicBox {
    /// Intrinsic width (px).
    pub width: f32,
    /// Intrinsic height (px).
    pub height: f32,
    /// Natural width divided by natural height, when known.
    pub aspect_ratio: Option<f32>,
}

impl IntrinsicBox {
    /// Constructs an intrinsic box from a decoded image's pixel dimensions.
    pub fn new(width: f32, height: f32) -> Self {
        Self {
            width,
            height,
            aspect_ratio: None,
        }
    }

    /// Sets the intrinsic ratio while preserving the concrete fallback size.
    pub fn with_aspect_ratio(mut self, aspect_ratio: f32) -> Self {
        self.aspect_ratio = Some(aspect_ratio);
        self
    }
}

/// Successful return type from the resolver.
#[derive(Debug, Clone)]
pub struct ResolvedIntrinsic {
    /// Intrinsic size and metadata.
    pub intrinsic: IntrinsicBox,
    /// Resolve disposition (`Ok` / `Fallback`).
    pub disposition: ResolveDisposition,
}

/// Classification of successful resolution or fallback.
#[non_exhaustive]
#[derive(Debug, Clone)]
pub enum ResolveDisposition {
    /// Normal successful resolution.
    Ok,
    /// Consumer intentionally chose a fallback (such as a placeholder for a missing image).
    /// Raikiri records this as `WarningKind::ResolverFallback` in
    /// `RenderSummary.warnings`.
    Fallback {
        /// Human-readable reason for the fallback.
        reason: String,
    },
}

/// Details of the element being resolved (borrowed reference).
///
/// `element_kind`/`hint_size`/`attributes` are unnecessary for this crate's
/// PNG-only `<img>` scope (YAGNI); populate them in the future when supporting
/// `<object>`, `<svg>`, etc.
#[non_exhaustive]
#[derive(Debug)]
pub struct ResolverRequest<'a> {
    url: &'a Url,
}

impl<'a> ResolverRequest<'a> {
    /// Constructs a request for the given absolute resource URL.
    pub fn new(url: &'a Url) -> Self {
        Self { url }
    }

    /// The resource URL to resolve.
    pub fn url(&self) -> &Url {
        self.url
    }
}

/// Resolver-layer error.
#[non_exhaustive]
#[derive(Debug)]
pub enum ResolverError {
    /// Failed to fetch bytes (`NetworkProvider::fetch` returned `Err`).
    Network(NetworkError),
    /// Failed to decode the fetched bytes (such as an invalid PNG).
    Decode(String),
}

impl std::fmt::Display for ResolverError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Network(e) => write!(f, "image fetch failed: {e}"),
            Self::Decode(msg) => write!(f, "image decode failed: {msg}"),
        }
    }
}

impl std::error::Error for ResolverError {}

#[cfg(test)]
mod tests;
