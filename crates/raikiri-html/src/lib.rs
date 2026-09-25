//! raikiri-html — HTML document entry points.
//!
//! - Parser layer: html5ever wrapper ([`RaikiriTreeSink`]) with the [`parse()`] /
//!   [`parse_with_sink`] entry points that produce a pre-cascade
//!   [`UncascadedDocument`].
//! - Document layer: [`parse_html`] / [`parse_html_with_limits`] /
//!   [`parse_html_with_resources`] assemble a cascaded [`HtmlDocument`];
//!   [`build_cascaded`] and friends expose the cascade orchestration.
//! - Render layer: [`render_streaming`] is the single page-streaming entry
//!   point; [`RenderOptions`] combines the consumer resource handoff
//!   ([`RenderResources`]) with page-event and consumer-property observers.

mod cascade;
mod document;
mod document_parse;
mod import;
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
pub use document_parse::{parse_html, parse_html_with_limits};
pub use parse::{effective_document_base_url, parse, parse_fragment, parse_with_sink};
pub use render::{RenderOptions, plan, render_streaming};
pub use resources::{
    DEFAULT_MAX_AGGREGATE_RESOURCE_BYTES, DEFAULT_MAX_RESOURCE_BYTES, RenderResources,
    ResourceLimits, parse_html_with_resources,
};
pub use sink::RaikiriTreeSink;
pub use types::{ParseOptions, UncascadedDocument};
pub use ua::MINIMAL_UA_CSS;

// Types that appear in the signatures above, re-exported so a consumer that
// depends only on this crate (plus `raikiri-dom` / `raikiri-traits`) can name
// them without depending on the style or text-layout implementation crates.
pub use parley::FontContext;
pub use raikiri_style::{
    CascadeResult, ConsumerPropertyGrammar, ConsumerPropertyRegistration, MediaContext,
    PageContextQuery,
};

#[cfg(test)]
mod document_tests;

#[cfg(test)]
#[allow(clippy::needless_lifetimes, clippy::collapsible_if)]
mod tests;
