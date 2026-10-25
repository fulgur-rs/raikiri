//! XML parsing into the same DOM and stylesheet pipeline as HTML.

use std::io::Read;

use raikiri_traits::{LimitKind, ParseError, RenderLimits};
use xml5ever::driver::{XmlParseOpts, parse_document};

use crate::parse::{dom_node_budget_error, parse_with_budgets_and_parser};
use crate::{ParseOptions, RaikiriTreeSink, UncascadedDocument};

mod subset;

use subset::SkipInternalSubset;

/// The deepest element nesting [`parse_xhtml`] accepts, the depth at which
/// libxml2 also stops by default. xml5ever looks through its stack of open
/// elements for every end tag, so unbounded nesting would make the parse time
/// quadratic in the input; real documents stay far below this.
const MAX_ELEMENT_DEPTH: u32 = 256;

/// Parse UTF-8 XHTML with xml5ever, preserving self-closing elements,
/// namespaces, case, and CDATA text, and collecting stylesheets as
/// [`crate::parse()`] does.
///
/// The input is read in chunks and handed to the parser as it arrives, so it
/// is never held in one buffer. The default input-byte, DOM-node, and
/// parse-warning budgets apply as they do for [`crate::parse()`], and
/// elements nested deeper than 256 levels return
/// [`LimitKind::DomDepth`]. A parse stops reading at the first chunk after
/// a budget is exceeded.
///
/// xml5ever recovers from errors instead of rejecting the document, so
/// malformed XML (an unclosed element, a mismatched end tag, an undeclared
/// prefix, a duplicate attribute, text outside the root element) is repaired
/// the way xml5ever repairs it and recorded as a parse warning. This never
/// falls back to HTML parsing. The XML declaration is kept as a processing
/// instruction. Namespace declarations (`xmlns`, `xmlns:p`) scope the element
/// and attribute names but are not kept as attributes.
///
/// DTDs are not interpreted. A `<!DOCTYPE>` is skipped, and the internal
/// subset in its brackets is dropped with a warning: it defines no entity, so
/// a document cannot make the parser expand one. A character reference name
/// is looked up in the HTML named character reference table (`&nbsp;`,
/// `&copy;`) and an unknown name stays literal text. External DTDs and
/// entities are never fetched.
///
/// # Errors
/// Returns an error for invalid UTF-8, input I/O failures, or exceeded
/// parser budgets.
pub fn parse_xhtml(
    input: impl Read,
    options: &ParseOptions<'_>,
) -> Result<UncascadedDocument, ParseError> {
    parse_with_limits(input, options, &RenderLimits::default())
}

fn parse_with_limits(
    input: impl Read,
    options: &ParseOptions<'_>,
    limits: &RenderLimits,
) -> Result<UncascadedDocument, ParseError> {
    let sink = RaikiriTreeSink::with_limits(limits.max_parse_warnings, limits.max_dom_nodes)
        .with_open_depth_limit(MAX_ELEMENT_DEPTH);
    let node_limit_signal = sink.dom_node_limit_signal();
    let depth_limit_signal = sink.open_depth_limit_signal();
    let parser = SkipInternalSubset::new(parse_document(sink, XmlParseOpts::default()));
    parse_with_budgets_and_parser(input, parser, options, limits.max_input_bytes, || {
        dom_node_budget_error(limits.max_dom_nodes, &node_limit_signal).or_else(|| {
            depth_limit_signal
                .get()
                .map(|actual| ParseError::LimitExceeded {
                    kind: LimitKind::DomDepth,
                    limit: u64::from(MAX_ELEMENT_DEPTH),
                    actual,
                })
        })
    })
}

#[cfg(test)]
mod tests;
