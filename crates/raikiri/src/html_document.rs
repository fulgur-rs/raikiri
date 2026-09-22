//! `HtmlDocument`: assembled document type。
//!
//! HTML を parse して cascade まで完了した document unit。Consumer 視点で
//! 「layout/paint に投入できる状態」を単一 handle で表現する。将来
//! raikiri-dom に `Document::assemble` が生えた時点で `pub use raikiri_dom::
//! Document` に透過的に置換される (Consumer surface 不変)。
//!
//! blitz `HtmlDocument` の analog (spec §L1134 blitz-compat 対応)。名前のみ
//! 一致、shape / code / UA CSS の持ち込みなし (independent implementation)。

use raikiri_html::UncascadedDocument;
use raikiri_style::{CascadeResult, FontFaceRegistry};
use url::Url;

/// Cascade まで完了した document unit。
#[derive(Debug)]
#[non_exhaustive]
pub struct HtmlDocument {
    pub(crate) uncascaded: UncascadedDocument,
    pub(crate) cascade: CascadeResult,
    pub(crate) font_faces: FontFaceRegistry,
    pub(crate) effective_base_url: Option<Url>,
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

    /// Parse 時に head/body から集約された inline `<style>` element と
    /// fetched head stylesheet source の list
    /// ([`UncascadedDocument::stylesheet_sources`] に一致)。
    /// HTML/XHTML と SVG の `<style>` は対象で、MathML の同名 element は除外される。
    pub fn stylesheet_sources(&self) -> &[String] {
        &self.uncascaded.stylesheet_sources
    }

    /// Effective document base URL used for linked stylesheets, imports, fonts,
    /// and relative replaced-element URLs.
    pub fn effective_base_url(&self) -> Option<&Url> {
        self.effective_base_url.as_ref()
    }
}
