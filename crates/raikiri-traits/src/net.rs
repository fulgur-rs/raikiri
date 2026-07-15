//! Network provider trait + neutral network types.
//!
//! blitz-traits::NetProvider の shape に揃える (Finding #6 対応)。raikiri は
//! sync core のため callback ではなく sync return。policy 適用は
//! `raikiri-net::SandboxedNetProvider` による wrap で行う。

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use bytes::Bytes;
use url::Url;

use crate::page::FormData;
use crate::policy::{PolicyViolation, ResourceKind};

/// HTTP header の list。同名 header の複数値も表現可能。
///
/// 軽量 shape に留めるため `http::HeaderMap` を採用せず `Vec<(String, String)>`。
/// M6 blitz-compat 実装時に必要なら再検討。
pub type HeaderMap = Vec<(String, String)>;

/// Consumer が実装する sync-return の network provider trait。
///
/// raikiri は timer thread を持たない (round 5 review #2)。timeout は
/// Consumer が自身の async runtime / thread pool で管理する。
///
/// Finding #6 対応 (round 7 未対応 finding: byte enforcement 戦略確定は
/// M4 sandboxed-net-provider-impl 前)。
pub trait NetworkProvider: Send + Sync {
    /// 1 fetch を同期実行し、結果か error を返す。
    ///
    /// `NetworkError` は spec §4 で `PolicyViolation(PolicyViolation)` variant を
    /// 直接持つため約 144 bytes となり、clippy::result_large_err の閾値 (128 bytes)
    /// を超える。API shape は §4 authoritative のため M1.1 では lint を suppress し、
    /// Box wrapper 化 (`PolicyViolation(Box<PolicyViolation>)`) の適用可否は M4
    /// sandboxed-net-provider-impl 段階で NetworkError 実利用と併せて再判断する。
    #[allow(clippy::result_large_err)]
    fn fetch(&self, request: Request) -> Result<FetchedResource, NetworkError>;
}

/// Fetch 要求の全情報。
#[derive(Debug, Clone)]
pub struct Request {
    /// Target URL (redirect 前)。
    pub url: Url,
    /// HTTP method。
    pub method: Method,
    /// Content-Type header (POST body 用)。
    pub content_type: Option<String>,
    /// 追加 header (`Accept`, `Referer`, custom 等)。
    pub headers: HeaderMap,
    /// Request body。
    pub body: Body,
    /// AbortSignal (option、Consumer が渡す)。
    pub signal: Option<AbortSignal>,
    /// raikiri 追加: fetch の目的 (blitz は doc_id、raikiri は context 表現)。
    pub kind: ResourceKind,
}

/// Fetch 成功時の response。
#[derive(Debug, Clone)]
pub struct FetchedResource {
    /// Response body の生 bytes。
    pub bytes: Bytes,
    /// `Content-Type` header (parse 済み MIME type)。
    pub content_type: Option<String>,
    /// Redirect 後の実効 URL。
    pub final_url: Url,
    /// 明示された character encoding (無ければ MIME や BOM から推測)。
    pub encoding: Option<String>,
}

/// Request body 表現。
#[non_exhaustive]
#[derive(Debug, Clone)]
pub enum Body {
    /// 生 bytes body。
    Bytes(Bytes),
    /// application/x-www-form-urlencoded body。
    Form(FormData),
    /// body 無し (GET 等)。
    Empty,
}

/// HTTP method (拡張余地あり、round 3 review #3 訂正: HTTP method は仕様上
/// 拡張可能なため `#[non_exhaustive]`)。
///
/// M1.1 では GET / POST のみ (fulgur の primary use case)。PUT / DELETE / PATCH /
/// HEAD / OPTIONS は M4 sandboxed-net-provider-impl / Consumer 実 use case 発生時
/// に追加。`#[non_exhaustive]` により Consumer 側 exhaustive match の accidental
/// break を防ぐ。
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Method {
    /// GET。
    Get,
    /// POST。
    Post,
}

/// blitz と同じ AbortSignal shape (AtomicBool ラッパ)。
///
/// Consumer が `AbortController::abort()` を呼ぶと、共有 AtomicBool が
/// `true` になり、raikiri と Consumer の両側から観測可能。
#[derive(Debug, Clone)]
pub struct AbortSignal(Arc<AtomicBool>);

impl AbortSignal {
    /// signal が abort されたか。
    pub fn is_aborted(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}

/// AbortSignal を produce する controller。round 5 review #2 対応: raikiri は
/// timer thread を一切 spawn しない。timeout は Consumer が自分の async
/// runtime で管理する。
#[derive(Debug, Default)]
pub struct AbortController {
    /// このコントローラが管理する signal。
    pub signal: AbortSignal,
}

impl AbortController {
    /// 新規 controller を生成。
    pub fn new() -> Self {
        Self {
            signal: AbortSignal(Arc::new(AtomicBool::new(false))),
        }
    }

    /// signal を abort 状態に遷移させる。
    pub fn abort(&self) {
        self.signal.0.store(true, Ordering::Release);
    }
}

impl Default for AbortSignal {
    fn default() -> Self {
        Self(Arc::new(AtomicBool::new(false)))
    }
}

/// Network 層 error。§4 の 5 variant を再現。
#[non_exhaustive]
#[derive(Debug)]
pub enum NetworkError {
    /// AbortSignal によって中断された。
    Aborted,
    /// ResourcePolicy 違反 (SandboxedNetProvider が発火)。
    PolicyViolation(PolicyViolation),
    /// I/O error。
    Io(std::io::Error),
    /// HTTP status code error (4xx / 5xx)。
    Http(u16),
    /// その他。
    Other(String),
}

impl std::fmt::Display for NetworkError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Aborted => write!(f, "Network fetch aborted"),
            Self::PolicyViolation(v) => {
                write!(f, "Network fetch violated policy: {:?}", v.violation_type)
            }
            Self::Io(_) => write!(f, "Network I/O error"),
            Self::Http(status) => write!(f, "Network HTTP status error: {status}"),
            Self::Other(msg) => write!(f, "Network error: {msg}"),
        }
    }
}

impl std::error::Error for NetworkError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::PolicyViolation(v) => Some(v),
            Self::Io(e) => Some(e),
            Self::Aborted | Self::Http(_) | Self::Other(_) => None,
        }
    }
}
