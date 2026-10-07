//! Public data types produced/consumed by raikiri-html.

use raikiri_dom::Document;
use raikiri_traits::{NetworkProvider, RenderWarning};
use url::Url;

/// The output of the parse phase: the pre-cascade DOM, collected inline/external
/// stylesheet sources, and parse warnings.
///
/// `stylesheet_sources` collects the text of `<style>` elements inside and outside
/// `<head>`, plus CSS text fetched from `<head>` links with `rel="stylesheet"`
/// via `ParseOptions::network`, all as Author stylesheets.
/// The parse layer fetches these after `TreeSink::finish()`, so the sink performs no I/O.
/// Head sources come first; inline styles outside the head follow in document order.
/// Inline and external sources retain this projection order. Leading `@import` rules
/// in inline, extra, and external stylesheets are expanded in place when a provider
/// is available. Imports that cannot be resolved, form cycles, or hit depth or
/// resource limits remain as original at-rules, and parsing continues.
/// `warnings` stores nonfatal html5ever tokenizer parse errors as
/// [`raikiri_traits::WarningKind::HtmlParseError`] and stylesheet fetch
/// failures as [`raikiri_traits::WarningKind::NetworkFallback`] or
/// [`raikiri_traits::WarningKind::PolicyWarning`].
#[derive(Debug)]
pub struct UncascadedDocument {
    /// DOM tree (raikiri-dom arena).
    pub dom: Document,
    /// CSS text from `<style>` elements inside and outside `<head>`, and successfully
    /// fetched `<link rel="stylesheet">` elements inside `<head>`. Head sources come
    /// first; inline styles outside the head follow in document order.
    /// The raikiri umbrella crate cascades them as Author-origin stylesheets via
    /// `build_cascaded`.
    pub stylesheet_sources: Vec<StylesheetSource>,
    /// Consumer-supplied sheets, cascaded with User origin before author sheets.
    pub user_stylesheet_sources: Vec<StylesheetSource>,
    /// Number of DOM-associated sheets preceding the parsed consumer sheets.
    /// Later calls to [`Document::add_stylesheet`] follow these consumer sheets.
    pub user_stylesheet_insertion_index: usize,
    /// Nonfatal html5ever parse errors, retained as warnings.
    /// The higher-level orchestrator (the raikiri umbrella crate) merges them into
    /// `RenderSummary.warnings` through `Document`.
    pub warnings: Vec<RenderWarning>,
    /// HTML5 quirks mode, using a raikiri-native enum that mirrors html5ever's
    /// `QuirksMode`. The cascade may use this for selector behavior and special
    /// rules.
    pub quirks_mode: raikiri_traits::QuirksMode,
}

/// One stylesheet root with imports resolved into source-ordered parts.
///
/// A part remains a top-level stylesheet, so its namespace declarations and
/// descriptor rules retain their original scope. The element media attribute
/// applies to every part, in conjunction with the part's import conditions.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StylesheetSource {
    /// Parts in the order they enter the cascade.
    pub parts: Vec<StylesheetPart>,
    /// The media query list from the owning style or link element.
    pub media: Option<String>,
}

impl StylesheetSource {
    /// Create a stylesheet before import resolution.
    pub fn new(source: String, media: Option<String>) -> Self {
        Self {
            parts: vec![StylesheetPart {
                source,
                media: Vec::new(),
            }],
            media,
        }
    }
}

/// Top-level CSS text guarded by the media lists of its importing ancestors.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StylesheetPart {
    /// CSS text parsed in its own stylesheet namespace scope.
    pub source: String,
    /// Media query lists combined with logical AND, preserving each list's OR.
    pub media: Vec<String>,
}

/// Options passed to the parse phase.
///
/// `parse_with_sink` consumes `extra_stylesheets` with User origin. It resolves
/// leading `@import` rules in inline,
/// extra, and external stylesheets when `network` is available. It also uses
/// `network` and `base_url` to fetch `<head>` links with `rel="stylesheet"` and
/// resolve relative URLs in stylesheet `@import` rules
/// (`parse.rs::fetch_external_stylesheets`). Failed fetches record a warning.
/// Imports for which no request can be made (such as a missing base or unsafe URL),
/// or which hit cycle, depth, or resource limits, remain opaque at-rules while
/// parsing continues. Fetching external resources for replaced elements
/// (such as `<img>`) remains outside this task's scope and is not yet implemented.
pub struct ParseOptions<'a> {
    /// CSS strings supplied by the consumer for cascading (such as fulgur's internal UA CSS).
    pub extra_stylesheets: &'a [&'a str],
    /// Provider used to fetch external resources such as replaced elements and
    /// `<link rel="stylesheet">`. With `None`, stylesheet links are not fetched
    /// and are ignored (external stylesheets are opt-in).
    /// The provider is accepted for replaced elements, but fetching them is not yet
    /// implemented (outside this scope).
    pub network: Option<&'a dyn NetworkProvider>,
    /// Base for resolving relative URLs. Relative `<link rel="stylesheet" href="...">`
    /// URLs are resolved with `Url::join`. If `base_url` is `None` and `href` is
    /// relative, the link cannot be resolved and is not fetched.
    pub base_url: Option<Url>,
}
