//! Resource policy trait + policy violation types.
//!
//! Policy consumed by `SandboxedNetProvider` / `SandboxedResolver` (raikiri-net).
//! Finding #6.

use std::time::Duration;
use url::Url;

/// Trait for policy decisions on resource fetches and decoding.
///
/// raikiri-net `SandboxedNetProvider<P>` / `SandboxedResolver<R>` call
/// its methods at the appropriate pre-fetch / post-fetch phase. Consumers
/// can implement a custom policy or use `raikiri-net::DefaultSandboxPolicy`.
///
/// Finding #6 (round 7 outstanding item: settle removal of redirect /
/// timeout / recursion methods before sandboxed-net-provider-impl).
pub trait ResourcePolicy: Send + Sync {
    /// Whether the URL scheme (`https` / `data` / `file` / ...) is allowed.
    fn is_scheme_allowed(&self, scheme: &str, kind: ResourceKind) -> bool;

    /// Whether the host is allowed.
    fn is_host_allowed(&self, host: &str, kind: ResourceKind) -> bool;

    /// Whether redirects are allowed.
    fn allow_redirect(&self, from: &Url, to: &Url, hop: u32) -> bool;

    /// Maximum number of redirect hops.
    fn max_redirect_hops(&self, kind: ResourceKind) -> u32;

    /// Maximum bytes before fetching (based on Content-Length; DoS mitigation).
    fn max_fetch_bytes(&self, kind: ResourceKind) -> Option<u64>;

    /// Maximum bytes after decoding (limit on expanded memory footprint).
    fn max_decoded_bytes(&self, kind: ResourceKind) -> Option<u64>;

    /// Timeout for the whole fetch (prevent thread hangs).
    fn fetch_timeout(&self, kind: ResourceKind) -> Duration;

    /// Timeout for decoding.
    fn decode_timeout(&self, kind: ResourceKind) -> Duration;

    /// Allowed MIME types (`text/css`, `image/png`, ...).
    fn allowed_mime_types(&self, kind: ResourceKind) -> Vec<String>;

    /// Maximum depth of chained `@import`.
    fn max_import_depth(&self) -> u32;

    /// Maximum depth of external SVG recursion.
    fn max_svg_recursion_depth(&self) -> u32;
}

/// Classification of fetched resources (context for policy decisions).
///
/// Finding #6. Reproduces the seven variants listed in §4. For future extension,
/// `#[non_exhaustive]` permits future extension.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ResourceKind {
    /// `@import` inside CSS.
    StylesheetImport,
    /// External stylesheet fetched from `<link rel="stylesheet">`.
    ExternalStylesheet,
    /// `<img src>`, `background-image`, etc.
    Image,
    /// `@font-face src`.
    Font,
    /// External SVG.
    Svg,
    /// External MathML.
    MathML,
    /// Fallback.
    Other,
}

impl std::fmt::Display for ResourceKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::StylesheetImport => write!(f, "stylesheet @import"),
            Self::ExternalStylesheet => write!(f, "external stylesheet"),
            Self::Image => write!(f, "image"),
            Self::Font => write!(f, "font"),
            Self::Svg => write!(f, "SVG"),
            Self::MathML => write!(f, "MathML"),
            Self::Other => write!(f, "other resource"),
        }
    }
}

/// Details of a policy violation.
#[derive(Debug, Clone)]
pub struct PolicyViolation {
    /// Resource kind for which the violation occurred.
    pub kind: ResourceKind,
    /// Target URL.
    pub url: Url,
    /// Violation type.
    pub violation_type: ViolationType,
    /// Human-readable details.
    pub details: String,
}

impl std::fmt::Display for PolicyViolation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Policy violation ({} {}) at {}: {}",
            self.kind, self.violation_type, self.url, self.details
        )
    }
}

impl std::error::Error for PolicyViolation {}

/// Classification of policy violations.
///
/// Reproduces the eight variants in §4. Round 7 outstanding item: redirect /
/// timeout / recursion methods may later be removed from `ResourcePolicy`.
#[non_exhaustive]
#[derive(Debug, Clone)]
pub enum ViolationType {
    /// URL scheme rejected by `is_scheme_allowed`.
    SchemeNotAllowed,
    /// Host rejected by `is_host_allowed`.
    HostNotAllowed,
    /// Redirect rejected by `allow_redirect`.
    RedirectDenied,
    /// Fetched bytes exceeded `max_fetch_bytes`.
    FetchTooLarge {
        /// The size limit that was exceeded.
        limit: u64,
        /// The actual size encountered.
        actual: u64,
    },
    /// Decoded bytes exceeded `max_decoded_bytes`.
    DecodedTooLarge {
        /// The size limit that was exceeded.
        limit: u64,
        /// The actual size encountered.
        actual: u64,
    },
    /// Fetch / decode timeout exceeded.
    Timeout,
    /// MIME type not in `allowed_mime_types`.
    MimeNotAllowed {
        /// The MIME type that was rejected.
        mime: String,
    },
    /// `@import` / SVG recursion depth exceeded.
    RecursionExceeded {
        /// The recursion depth that exceeded the limit.
        depth: u32,
    },
    /// The resolved connection address failed the non-overridable
    /// SSRF safety floor (private, loopback, link-local, CGNAT, or
    /// metadata-range destination). Distinct from `HostNotAllowed`,
    /// which is a Consumer-configured `ResourcePolicy` decision — this
    /// variant fires regardless of policy configuration.
    PrivateNetworkBlocked,
    /// Other violation.
    Other,
}

impl std::fmt::Display for ViolationType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SchemeNotAllowed => write!(f, "scheme not allowed"),
            Self::HostNotAllowed => write!(f, "host not allowed"),
            Self::RedirectDenied => write!(f, "redirect denied"),
            Self::FetchTooLarge { limit, actual } => {
                write!(f, "fetch too large (limit={limit}, actual={actual})")
            }
            Self::DecodedTooLarge { limit, actual } => {
                write!(
                    f,
                    "decoded content too large (limit={limit}, actual={actual})"
                )
            }
            Self::Timeout => write!(f, "timeout exceeded"),
            Self::MimeNotAllowed { mime } => write!(f, "MIME type not allowed: {mime}"),
            Self::RecursionExceeded { depth } => {
                write!(f, "recursion depth exceeded ({depth})")
            }
            Self::PrivateNetworkBlocked => {
                write!(f, "destination is a private network address")
            }
            Self::Other => write!(f, "unspecified policy violation"),
        }
    }
}

#[cfg(test)]
mod tests;
