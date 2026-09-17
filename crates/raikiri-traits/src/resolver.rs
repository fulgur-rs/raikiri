//! Replaced-element resolver trait + resolve types.
//!
//! Consumer が `<img>`, `<object>`, `<embed>`, `<svg>` 等の replaced element の
//! intrinsic size を返す trait。size 決定 だけを扱う (fetch は Consumer 側)。

use url::Url;

use crate::net::NetworkError;

/// Replaced element の intrinsic size を resolve する Consumer 側 trait。
///
/// - req に含まれる URL の scheme / host / size 等の検証は Consumer 責任。
///   集中的に policy を効かせたい場合は `raikiri-net::SandboxedResolver` で wrap
///   (Finding #6 対応、§10 参照)。
/// - round 4 review #3 対応: `Err` は常に terminal (`RenderError::Resolver`)。
///   Consumer 側 fallback は `Ok(ResolvedIntrinsic { intrinsic, disposition:
///   Fallback { .. } })` として返し、raikiri は disposition を見て
///   `RenderSummary.warnings` に自動記録する。
pub trait ReplacedResolver {
    /// 1 replaced element の resolve。
    ///
    /// `ResolverError::Network(NetworkError)` は `NetworkError` 自体が
    /// `PolicyViolation` payload を直接保有するため約 144 bytes となり、
    /// clippy::result_large_err の閾値 (128 bytes) を超える。
    /// Box wrapper 化の適用可否は実装段階で再判断する。
    #[allow(clippy::result_large_err)]
    fn resolve(&self, req: ResolverRequest<'_>) -> Result<ResolvedIntrinsic, ResolverError>;
}

/// Intrinsic size。
#[derive(Debug, Default, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub struct IntrinsicBox {
    /// Intrinsic width (px).
    pub width: f32,
    /// Intrinsic height (px).
    pub height: f32,
}

impl IntrinsicBox {
    /// Constructs an intrinsic box from a decoded image's pixel dimensions.
    pub fn new(width: f32, height: f32) -> Self {
        Self { width, height }
    }
}

/// Resolver からの正常 return 型。
#[derive(Debug, Clone)]
pub struct ResolvedIntrinsic {
    /// Intrinsic size + metadata。
    pub intrinsic: IntrinsicBox,
    /// Resolve disposition (`Ok` / `Fallback`)。
    pub disposition: ResolveDisposition,
}

/// Resolve 成功 / fallback の分類。
#[non_exhaustive]
#[derive(Debug, Clone)]
pub enum ResolveDisposition {
    /// 通常の resolve 成功。
    Ok,
    /// Consumer が意図的に fallback を選択 (image not found → placeholder 等)。
    /// raikiri は `WarningKind::ResolverFallback` として
    /// `RenderSummary.warnings` に記録する。
    Fallback {
        /// 人間可読な fallback 理由。
        reason: String,
    },
}

/// Resolve 対象 element の詳細 (borrowed reference)。
///
/// `element_kind`/`hint_size`/`attributes` は本 crate の `<img>` PNG-only
/// スコープでは不要なため未追加 (YAGNI) — 将来 `<object>`/`<svg>` 等に
/// 対応する際に populate する。
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

/// Resolver 層 error。
#[non_exhaustive]
#[derive(Debug)]
pub enum ResolverError {
    /// Byte 取得に失敗した (`NetworkProvider::fetch` が `Err` を返した)。
    Network(NetworkError),
    /// 取得した byte 列のデコードに失敗した (不正な PNG 等)。
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
mod tests {
    use super::*;

    #[test]
    fn resolver_request_carries_url() {
        let url = Url::parse("file:///tmp/x.png").unwrap();
        let req = ResolverRequest::new(&url);
        assert_eq!(req.url(), &url);
    }

    #[test]
    fn intrinsic_box_carries_dimensions() {
        let b = IntrinsicBox::new(64.0, 32.0);
        assert_eq!((b.width, b.height), (64.0, 32.0));
    }

    #[test]
    fn resolver_error_network_and_decode_display() {
        let net_err = NetworkError::Other("boom".into());
        let e = ResolverError::Network(net_err);
        assert!(format!("{e}").contains("boom"));

        let e = ResolverError::Decode("bad PNG".into());
        assert_eq!(format!("{e}"), "image decode failed: bad PNG");
    }
}
