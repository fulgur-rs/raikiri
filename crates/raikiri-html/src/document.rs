//! `HtmlDocument`: assembled document type.
//!
//! A document unit that has been parsed from HTML and cascaded. From a consumer's
//! perspective, one handle represents a document ready for layout and painting.
//!
//! Analogous to blitz `HtmlDocument` (spec §L1134 blitz compatibility). Only the name
//! matches; no shape, code, or UA CSS is imported (independent implementation).

use crate::UncascadedDocument;
use raikiri_style::{CascadeResult, FontFaceRegistry};
use url::Url;

/// A document unit after cascading.
#[derive(Debug)]
#[non_exhaustive]
pub struct HtmlDocument {
    pub(crate) uncascaded: UncascadedDocument,
    pub(crate) cascade: CascadeResult,
    pub(crate) font_faces: FontFaceRegistry,
    pub(crate) effective_base_url: Option<Url>,
}

impl HtmlDocument {
    /// Reference to the DOM tree (entry point for using the Dom / Element traits).
    pub fn dom(&self) -> &raikiri_dom::Document {
        &self.uncascaded.dom
    }

    /// Cascade result (per-node ComputedValues).
    pub fn cascade(&self) -> &CascadeResult {
        &self.cascade
    }

    /// Inline `<style>` elements collected from head/body during parsing and
    /// a list of fetched head stylesheet sources
    /// (matching [`UncascadedDocument::stylesheet_sources`]).
    /// Includes HTML/XHTML and SVG `<style>`, but excludes MathML elements with that name.
    pub fn stylesheet_sources(&self) -> &[crate::StylesheetSource] {
        &self.uncascaded.stylesheet_sources
    }

    /// Effective document base URL used for linked stylesheets, imports, fonts,
    /// and relative replaced-element URLs.
    pub fn effective_base_url(&self) -> Option<&Url> {
        self.effective_base_url.as_ref()
    }

    /// Take ownership of the parsed document and its cascade.
    ///
    /// Single-page layout drivers mutate the DOM in place (layout results are
    /// stored on its nodes); they use this to obtain an owned
    /// [`UncascadedDocument`] together with the matching [`CascadeResult`].
    pub fn into_parts(self) -> (UncascadedDocument, CascadeResult) {
        (self.uncascaded, self.cascade)
    }
}
