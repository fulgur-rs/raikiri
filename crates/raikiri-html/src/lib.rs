//! raikiri-html — HTML document entry points.
//!
//! - Parser layer: html5ever wrapper ([`RaikiriTreeSink`]) with the [`parse()`] /
//!   [`parse_with_sink`] entry points that produce a pre-cascade
//!   [`UncascadedDocument`].
//! - Document layer: [`parse_html`] / [`parse_html_with_limits`] /
//!   [`parse_html_with_resources`] assemble a cascaded [`HtmlDocument`];
//!   [`build_cascaded`] and friends expose the cascade orchestration.
//! - Layout layer: [`layout`] returns an owned [`DocumentLayout`] with
//!   borrowed [`Page`] and [`Fragment`] views for drawing consumers.
//! - Inline layout: paragraphs are laid out by the shodo inline engine with
//!   the installed fonts, or with a font set built by
//!   [`FontCollectionBuilder`] and passed with [`RenderResources::fonts`].

mod cascade;
mod document;
mod document_layout;
mod document_parse;
mod font_collection;
mod import;
pub use import::expand_live_stylesheet_imports;
mod parse;
mod render;
mod resources;
mod sink;
mod types;
pub mod ua;

pub use cascade::{
    build_cascaded, build_cascaded_for_page, build_cascaded_with_consumer_properties,
    build_cascaded_with_media_context, build_cascaded_with_media_context_for_page,
    build_cascaded_with_media_context_for_page_and_consumer_properties, build_rule_tree,
    build_rule_tree_with_consumer_properties,
};
pub use document::HtmlDocument;
pub use document_layout::{
    Anchor, AnchorIndex, ClipKind, DecorationKind, DecorationLine, DecorationStyle, DocumentLayout,
    DomView, FontBlob, FontId, FontRef, FontVariation, Fragment, FragmentKind, GeneratedKind,
    Glyph, LayoutOptions, LayoutStatus, Link, Page, PageGeometry, PageMode, PaintEvent,
    PositionedGlyphRun, RepeatKind, RunSource, Synthesis, Tag, layout,
};
pub use document_parse::{parse_html, parse_html_with_limits};
pub use font_collection::{
    BundledFont, FontCollectionBuildError, FontCollectionBuilder, MAX_BUNDLED_FONT_BYTES,
    RenderFonts,
};
pub use parse::{effective_document_base_url, parse, parse_fragment, parse_with_sink};
pub use resources::{
    DEFAULT_MAX_AGGREGATE_RESOURCE_BYTES, DEFAULT_MAX_RESOURCE_BYTES, RenderResources,
    ResourceLimits, parse_html_with_resources,
};
pub use sink::RaikiriTreeSink;
pub use types::{ParseOptions, StylesheetPart, StylesheetSource, UncascadedDocument};
pub use ua::MINIMAL_UA_CSS;

// Types that appear in the signatures above, re-exported so a consumer that
// depends only on this crate (plus `raikiri-dom` / `raikiri-traits`) can name
// them without depending on the style or text-layout implementation crates.
pub use raikiri_style::{
    CascadeResult, ComputedValues, ConsumerPropertyGrammar, ConsumerPropertyRegistration,
    Declaration, MediaContext, MediaType, Origin, PageBleed, PageCascadeResult, PageContextQuery,
    PageMarginBoxCascadeResult, PageMarginBoxSlot, PageMarks, PageOrientation, PageSize,
    PageSizeKeyword,
};
// `PageCascadeResult` (see `Page::page_style`) reports declarations keyed and
// valued by these.
pub use raikiri_style::property::{PropertyKey, PropertyValue};

/// The computed-value types read from [`ComputedValues`] (for example through
/// `Page::computed`), named in the computed layer so a painter can annotate
/// every field it reads without depending on the style crate.
pub mod computed {
    pub use raikiri_style::computed_api::*;
}
pub use shodo::font::FontCollection;

pub use raikiri_traits::{
    ConsumerPropertyEvent, ConsumerPropertyObserver, LayoutConfig, LayoutConfigBuilder, NodeId,
    NodeKind, PageDefaults, PaintInsets, PaintRect, RenderError, RenderWarning, WarningKind,
};

#[cfg(test)]
mod document_tests;

#[cfg(test)]
#[allow(clippy::needless_lifetimes, clippy::collapsible_if)]
mod tests;
