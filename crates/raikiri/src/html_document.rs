//! `HtmlDocument`: M1 assembled document type (raikiri-spike-m1.11)。
//!
//! HTML を parse して cascade まで完了した document unit。Consumer 視点で
//! 「layout/paint に投入できる状態」を単一 handle で表現する。M2+ で
//! raikiri-dom に `Document::assemble` が生えた時点で `pub use raikiri_dom::
//! Document` に透過的に置換される (Consumer surface 不変)。
//!
//! blitz `HtmlDocument` の analog (spec §L1134 blitz-compat 対応)。名前のみ
//! 一致、shape / code / UA CSS の持ち込みなし (memory
//! `raikiri-implementation-independence` 準拠)。

use raikiri_html::UncascadedDocument;
use raikiri_style::CascadeResult;

/// Cascade まで完了した document unit。
#[non_exhaustive]
pub struct HtmlDocument {
    pub(crate) uncascaded: UncascadedDocument,
    pub(crate) cascade: CascadeResult,
}

impl std::fmt::Debug for HtmlDocument {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HtmlDocument")
            .field("uncascaded", &self.uncascaded)
            .field("cascade", &"<CascadeResult>")
            .finish()
    }
}

impl HtmlDocument {
    /// DOM tree への参照 (Dom / Element trait を使う際の entry point)。
    pub fn dom(&self) -> &raikiri_dom::Document {
        &self.uncascaded.dom
    }

    /// Cascade 結果 (per-node ComputedValues)。
    pub fn cascade(&self) -> &CascadeResult {
        &self.cascade
    }

    /// Parse 時に head 配下から集約された `<style>` element の source list
    /// (M1 契約、[`UncascadedDocument::stylesheet_sources`] に一致)。
    /// `<body>` 内 `<style>` は M1 未対応 (M2+ で拡張予定、m1.23 契約継承)。
    pub fn stylesheet_sources(&self) -> &[String] {
        &self.uncascaded.stylesheet_sources
    }
}
