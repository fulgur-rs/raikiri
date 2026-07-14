//! Resource policy trait + policy violation types.
//!
//! `SandboxedNetProvider` / `SandboxedResolver` (raikiri-net) が消費する policy。
//! Finding #6 対応。

use std::time::Duration;
use url::Url;

/// Resource fetch / decode に対する policy 判定 trait。
///
/// raikiri-net の `SandboxedNetProvider<P>` / `SandboxedResolver<R>` が
/// 各 method を pre-fetch / post-fetch phase で呼び分ける。Consumer が
/// custom policy を実装するか、`raikiri-net::DefaultSandboxPolicy` を利用。
///
/// Finding #6 対応 (round 7 未対応 finding: redirect / timeout / recursion 系
/// method の削除は M4 sandboxed-net-provider-impl 前に確定)。
pub trait ResourcePolicy: Send + Sync {
    /// URL scheme (`https` / `data` / `file` / ...) が許可されているか。
    fn is_scheme_allowed(&self, scheme: &str, kind: ResourceKind) -> bool;

    /// host が許可されているか。
    fn is_host_allowed(&self, host: &str, kind: ResourceKind) -> bool;

    /// redirect を許可するか。
    fn allow_redirect(&self, from: &Url, to: &Url, hop: u32) -> bool;

    /// redirect の最大 hop 数。
    fn max_redirect_hops(&self, kind: ResourceKind) -> u32;

    /// fetch 前の最大 byte 数 (Content-Length ベース、DoS 対策)。
    fn max_fetch_bytes(&self, kind: ResourceKind) -> Option<u64>;

    /// decode 後の最大 byte 数 (展開後 memory footprint 対策)。
    fn max_decoded_bytes(&self, kind: ResourceKind) -> Option<u64>;

    /// fetch 全体の timeout (thread hang 対策)。
    fn fetch_timeout(&self, kind: ResourceKind) -> Duration;

    /// decode の timeout。
    fn decode_timeout(&self, kind: ResourceKind) -> Duration;

    /// 許可される MIME type list (`text/css`, `image/png`, ...)。
    fn allowed_mime_types(&self, kind: ResourceKind) -> Vec<String>;

    /// chained `@import` の最大 depth。
    fn max_import_depth(&self) -> u32;

    /// 外部 SVG recursion の最大 depth。
    fn max_svg_recursion_depth(&self) -> u32;
}

/// Fetch した resource の分類 (policy 判定の context)。
///
/// Finding #6 対応。§4 に列挙された 7 variant を再現。将来拡張のため
/// `#[non_exhaustive]`。
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ResourceKind {
    /// `@import` inside CSS。
    StylesheetImport,
    /// `<link rel="stylesheet">` から fetch する外部 stylesheet。
    ExternalStylesheet,
    /// `<img src>`, `background-image` 等。
    Image,
    /// `@font-face src`。
    Font,
    /// 外部 SVG。
    Svg,
    /// 外部 MathML。
    MathML,
    /// Fallback。
    Other,
}

/// Policy 違反の詳細情報。
#[derive(Debug, Clone)]
pub struct PolicyViolation {
    /// どの resource kind で発生した違反か。
    pub kind: ResourceKind,
    /// 対象 URL。
    pub url: Url,
    /// 違反 type。
    pub violation_type: ViolationType,
    /// 人間可読な詳細 message。
    pub details: String,
}

/// Policy 違反の分類。
///
/// §4 の 8 variant を再現。round 7 未対応 finding: redirect / timeout /
/// recursion 系は M4 で `ResourcePolicy` から削除される可能性あり。
#[non_exhaustive]
#[derive(Debug, Clone)]
pub enum ViolationType {
    /// URL scheme が `is_scheme_allowed` で reject。
    SchemeNotAllowed,
    /// host が `is_host_allowed` で reject。
    HostNotAllowed,
    /// redirect が `allow_redirect` で reject。
    RedirectDenied,
    /// fetch 済 byte 数が `max_fetch_bytes` 超過。
    FetchTooLarge {
        /// The size limit that was exceeded.
        limit: u64,
        /// The actual size encountered.
        actual: u64,
    },
    /// decode 済 byte 数が `max_decoded_bytes` 超過。
    DecodedTooLarge {
        /// The size limit that was exceeded.
        limit: u64,
        /// The actual size encountered.
        actual: u64,
    },
    /// fetch / decode timeout 超過。
    Timeout,
    /// MIME type が `allowed_mime_types` に無い。
    MimeNotAllowed {
        /// The MIME type that was rejected.
        mime: String,
    },
    /// `@import` / SVG recursion depth 超過。
    RecursionExceeded {
        /// The recursion depth that exceeded the limit.
        depth: u32,
    },
    /// その他。
    Other,
}
