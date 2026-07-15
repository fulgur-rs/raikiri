//! Replaced-element resolver trait + resolve types.
//!
//! Consumer が `<img>`, `<object>`, `<embed>`, `<svg>` 等の replaced element の
//! intrinsic size を返す trait。size 決定 だけを扱う (fetch は Consumer 側)。

use std::marker::PhantomData;

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
    fn resolve(&self, req: ResolverRequest<'_>) -> Result<ResolvedIntrinsic, ResolverError>;
}

/// Intrinsic size + metadata。M4 で fields を populate。M1.1 では opaque。
#[allow(missing_docs)]
#[derive(Debug, Default, Clone)]
#[non_exhaustive]
pub struct IntrinsicBox {
    // M4 で populate:
    //   pub width: Option<f32>,
    //   pub height: Option<f32>,
    //   pub aspect_ratio: Option<f32>,
    //   pub baseline: Option<f32>,
    //   pub encoding: MediaEncoding,
    //   ...
}

impl IntrinsicBox {
    /// M1.1 placeholder constructor.
    pub fn new() -> Self {
        Self::default()
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
/// M4 で fields (element_kind / url / hint_size / attributes 等) を populate。
/// M1.1 では phantom lifetime marker のみ。
#[allow(missing_docs)]
#[derive(Debug)]
#[non_exhaustive]
pub struct ResolverRequest<'a> {
    // M4 で populate:
    //   pub url: &'a Url,
    //   pub element_kind: ReplacedElementKind,
    //   pub hint_size: Option<Size>,
    //   pub attributes: &'a Attributes,
    _marker: PhantomData<&'a ()>,
}

impl<'a> ResolverRequest<'a> {
    /// M1.1 placeholder constructor.
    pub fn new() -> Self {
        Self {
            _marker: PhantomData,
        }
    }
}

impl<'a> Default for ResolverRequest<'a> {
    fn default() -> Self {
        Self::new()
    }
}

/// Resolver 層 error。M4 で variant を populate。M1.1 では uninhabited。
///
/// M4 想定 variant:
///   - `Io(std::io::Error)`
///   - `Decode(String)`
///   - `Timeout`
///   - `NotSupported { kind: ReplacedElementKind }`
#[non_exhaustive]
#[derive(Debug)]
pub enum ResolverError {
    // M4 で populate。
}

impl std::fmt::Display for ResolverError {
    fn fmt(&self, _f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match *self {}
    }
}

impl std::error::Error for ResolverError {}
