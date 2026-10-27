//! Public parse entrypoints.

mod fragment;

use std::borrow::Cow;
use std::cell::Cell;
use std::io::Read;

#[cfg(test)]
use html5ever::driver::parse_fragment as parse_html_fragment;
use html5ever::driver::{ParseOpts, parse_document};
use html5ever::interface::QualName;
use html5ever::tendril::{TendrilSink, fmt::UTF8};
use html5ever::tree_builder::TreeSink;
use markup5ever::{LocalName, Namespace};
use raikiri_traits::{
    LimitKind, NetworkError, ParseError, RenderLimits, RenderWarning, ResourceKind, StylesheetKind,
    WarningKind,
};
use url::Url;

use crate::fetch::{
    get_request, network_error_summary, policy_violation_warning_kind, redacted_url, resolve_url,
};
use crate::import::{ImportBudget, expand_stylesheet_imports_with_budget};
use crate::input::Utf8Feed;
use crate::sink::RaikiriTreeSink;
use crate::types::{ParseOptions, StylesheetSource, UncascadedDocument};

/// Parse HTML and return an [`UncascadedDocument`] containing the pre-cascade
/// DOM, extracted inline `<style>` elements, and parse warnings.
///
/// Treat `input` as a UTF-8 byte stream. Read failures return
/// [`ParseError::Io`]; input that is not valid UTF-8 returns [`ParseError::Encoding`].
/// The default input-byte and DOM-node budgets return
/// [`ParseError::LimitExceeded`] when reached. This is the current scope;
/// encoding_rs support may be added later.
///
/// # Example
///
/// ```
/// use raikiri_html::{parse, ParseOptions};
/// use raikiri_traits::Dom;
///
/// let html = b"<html><body>Hi</body></html>";
/// let opts = ParseOptions::new();
/// let doc = parse(&html[..], &opts).unwrap();
/// // Parse succeeded; dom has root
/// assert_eq!(doc.dom.root_id().0, 0);
/// ```
pub fn parse<R: Read>(
    input: R,
    options: &ParseOptions<'_>,
) -> Result<UncascadedDocument, ParseError> {
    let limits = RenderLimits::default();
    parse_with_budgets(input, options, &limits)
}

/// Parse through a consumer-supplied sink. Consumer wrappers must declare
/// `type Output = UncascadedDocument` and propagate the inner sink's
/// `finish(self)` result.
///
/// Input reads use the default `max_input_bytes` cap. A caller-supplied sink
/// owns its DOM allocation policy. The public `RaikiriTreeSink::new` is a raw
/// sink without a node budget, so callers must supply another allocation
/// boundary or use [`parse`] for the built-in sink with both default caps.
///
/// After parsing, inject the default UA CSS into the Document with
/// [`raikiri_dom::Document::add_stylesheet`] and retain consumer extra sheets
/// with User origin. Leading `@import` rules in extra
/// and inline stylesheets expand in source order when `options.network` is set.
/// Then fetch external stylesheets in `<head>` via `options.network` and
/// `options.base_url`, merging successful responses into
/// `UncascadedDocument.stylesheet_sources` as Author after head sources
/// (see `fetch_external_stylesheets`). External stylesheets resolve nested
/// imports against the response `final_url`. Failed imports keep their original
/// at-rules; fetch failures record `NetworkFallback` or `PolicyWarning`.
pub fn parse_with_sink<R, S>(
    input: R,
    sink: S,
    options: &ParseOptions<'_>,
) -> Result<UncascadedDocument, ParseError>
where
    R: Read,
    S: TreeSink<Handle = usize, Output = UncascadedDocument>,
{
    let mut feed = Utf8Feed::new(parse_document(sink, ParseOpts::default()));
    feed.feed_reader_limited(input, RenderLimits::default().max_input_bytes, |_| true)?;
    finish_document(feed.finish()?, options)
}

pub(crate) fn parse_with_budgets<R: Read>(
    input: R,
    options: &ParseOptions<'_>,
    limits: &RenderLimits,
) -> Result<UncascadedDocument, ParseError> {
    let sink = RaikiriTreeSink::with_limits(limits.max_parse_warnings, limits.max_dom_nodes);
    let node_limit_signal = sink.dom_node_limit_signal();
    let parser = parse_document(sink, ParseOpts::default());
    parse_with_budgets_and_parser(input, parser, options, limits.max_input_bytes, || {
        dom_node_budget_error(limits.max_dom_nodes, &node_limit_signal)
    })
}

/// Reads `input` through `parser` and finishes the document, stopping at the
/// first input chunk after `budget_error` reports a tripped budget and
/// returning that error. The input-byte budget is enforced while reading.
/// Shared by the HTML and XHTML entry points.
pub(crate) fn parse_with_budgets_and_parser<R, P>(
    input: R,
    parser: P,
    options: &ParseOptions<'_>,
    max_input_bytes: Option<u64>,
    budget_error: impl Fn() -> Option<ParseError>,
) -> Result<UncascadedDocument, ParseError>
where
    R: Read,
    P: TendrilSink<UTF8, Output = UncascadedDocument>,
{
    let mut feed = Utf8Feed::new(parser);
    feed.feed_reader_limited(input, max_input_bytes, |_| budget_error().is_none())?;
    if let Some(error) = budget_error() {
        return Err(error);
    }
    let document = feed.finish()?;
    if let Some(error) = budget_error() {
        return Err(error);
    }
    finish_document(document, options)
}

/// The DOM-node budget error once the sink's signal reports a node over it.
pub(crate) fn dom_node_budget_error(
    limit: Option<u64>,
    signal: &Cell<Option<u64>>,
) -> Option<ParseError> {
    signal.get().map(|actual| ParseError::LimitExceeded {
        kind: LimitKind::DomNodes,
        limit: limit.unwrap_or_else(|| actual.saturating_sub(1)),
        actual,
    })
}

/// Parse HTML markup using the browser's fragment algorithm for an element context.
///
/// `context_local_name` and `context_namespace` must describe the target element.
/// For HTML elements, use the HTML namespace URI. The result's document root
/// contains the parsed fragment children, ready to copy into a live element or
/// template-content fragment.
pub fn parse_fragment<R: Read>(
    input: R,
    options: &ParseOptions<'_>,
    context_local_name: &str,
    context_namespace: &str,
    context_element_allows_scripting: bool,
) -> Result<UncascadedDocument, ParseError> {
    parse_fragment_with_limits(
        input,
        options,
        context_local_name,
        context_namespace,
        context_element_allows_scripting,
        &RenderLimits::default(),
        None,
    )
}

/// Parse an HTML fragment with explicit input-byte, warning, and node limits.
///
/// `limits.max_dom_nodes` retains its total-arena meaning, including the
/// virtual document root and all parser wrappers. `remaining_nodes` supplies
/// a separate fragment allocation budget: it includes template-content roots,
/// but excludes the parser's document, context element (and its template
/// contents), and synthetic HTML root. Both limits apply when supplied.
/// Detached nodes discarded during error recovery still consume both budgets.
/// Arena allocation stops at the cap; tree building stops after the current
/// token, and input processing stops by the next tag or bounded read chunk.
/// This is a node-count bound, not a byte-memory bound.
pub fn parse_fragment_with_limits<R: Read>(
    input: R,
    options: &ParseOptions<'_>,
    context_local_name: &str,
    context_namespace: &str,
    context_element_allows_scripting: bool,
    limits: &RenderLimits,
    remaining_nodes: Option<u64>,
) -> Result<UncascadedDocument, ParseError> {
    let context_name = QualName::new(
        None,
        Namespace::from(context_namespace),
        LocalName::from(context_local_name),
    );
    // html5ever allocates these wrappers before consuming any input.
    let overhead = 3 + u64::from(
        context_local_name == "template" && context_namespace == "http://www.w3.org/1999/xhtml",
    );
    let fragment_arena_limit = remaining_nodes.map(|limit| limit.saturating_add(overhead));
    let arena_limit = match (limits.max_dom_nodes, fragment_arena_limit) {
        (Some(total), Some(fragment)) => Some(total.min(fragment)),
        (total, fragment) => total.or(fragment),
    };
    let sink = RaikiriTreeSink::with_limits(limits.max_parse_warnings, arena_limit);
    let node_limit_signal = sink.dom_node_limit_signal();
    let parser =
        fragment::BudgetedFragmentParser::new(sink, context_name, context_element_allows_scripting);
    let mut doc =
        parse_with_budgets_and_parser(input, parser, options, limits.max_input_bytes, || {
            node_limit_signal
                .get()
                .map(|actual| ParseError::LimitExceeded {
                    kind: LimitKind::DomNodes,
                    limit: arena_limit.unwrap_or(u64::MAX),
                    actual,
                })
        })?;
    flatten_fragment_root(&mut doc.dom);
    Ok(doc)
}

fn flatten_fragment_root(document: &mut raikiri_dom::Document) {
    let root = document.root_index();
    let Some(root_node) = document.get_node(root) else {
        return; // cov:ignore: Document::root_index always names the allocated root node.
    };
    if root_node.children.len() != 1 {
        return;
    }
    let html = root_node.children[0];
    if document.get_node(html).and_then(|node| node.tag_name()) != Some("html")
        || document.element_namespace_uri(html) != Some("http://www.w3.org/1999/xhtml")
    {
        return;
    }
    document.reparent_children(html, root);
    document.detach_from_parent(html);
}

pub(crate) fn finish_document(
    mut doc: UncascadedDocument,
    options: &ParseOptions<'_>,
) -> Result<UncascadedDocument, ParseError> {
    // Inject the default UA CSS into the Document.
    doc.dom.add_stylesheet(
        Cow::Borrowed(crate::ua::MINIMAL_UA_CSS),
        StylesheetKind::UserAgent,
    );

    doc.user_stylesheet_insertion_index = doc.dom.stylesheets().count();

    // The HTML document base URL also resolves relative `@import` rules in
    // inline and extra stylesheets. External stylesheets instead use their
    // fetched `FetchedResource::final_url` as the base.
    let effective_base = effective_document_base_url(&doc, options.base_url.as_ref());
    // Paint resolves the relative `url()` values that computed values keep,
    // such as those of `style` attributes, against this base.
    doc.dom.set_base_url(effective_base.clone());
    // Share import limits across every stylesheet root in this document. A
    // separate per-root expander would let many inline/link sheets multiply
    // the fetch and expansion caps.
    let mut import_budget = ImportBudget::default();

    // Keep consumer sheets separate from author styles and retain import
    // conditions without flattening them through the DOM string API.
    for extra in options.extra_stylesheets {
        let expanded = expand_stylesheet_imports_with_budget(
            extra,
            effective_base.as_ref(),
            None,
            options.network,
            &mut doc.warnings,
            &mut import_budget,
        );
        doc.user_stylesheet_sources.push(StylesheetSource {
            parts: expanded,
            media: None,
        });
    }

    // Find `<link rel="stylesheet" href="...">` elements, fetch via
    // ParseOptions::network, and merge the CSS text into
    // doc.stylesheet_sources as Author stylesheet sources.
    fetch_external_stylesheets(&mut doc, options, &mut import_budget);

    Ok(doc)
}

/// Process stylesheet-bearing `<head>` elements in document order, adding
/// successfully fetched external CSS to `doc.stylesheet_sources` as Author.
/// Inline `<style>` elements and `<link>` elements in the head retain head order;
/// inline `<style>` elements outside the head follow. The cascade consumes
/// the resolved parts as Author sheets with their media conditions.
///
/// `sink::collect_head_stylesheet_sources` detects hrefs using only the
/// `Document` after `finish()`, with no side effects. Actual fetching happens
/// outside `TreeSink::finish()`, keeping the sink free of I/O. If
/// `options.network` is `None`, external stylesheets are ignored. Fetch failures
/// are nonfatal: record them in `doc.warnings` and continue parsing.
///
/// # Known scope limits
///
/// - DOM-associated sheets and consumer extra sheets cascade before head
///   stylesheets. Imports in extra sheets expand before this pass.
/// - **`disabled` / `crossorigin` / `integrity`**:
///   See the `sink::collect_external_stylesheet_hrefs` documentation. The
///   `media` attribute is recorded in each stylesheet and evaluated by
///   the cascade.
/// - **`<base>` search scope and href handling**:
///   See the `sink::find_document_base_href` documentation. Document-level
///   security policy for the frozen base URL algorithm is not implemented here.
/// - **Relative order of `<base>` and `<link>`**: fetching happens after
///   parsing finishes, so links before a later `<base>` also use the final base URL.
/// - **encoding**: CSS assumes UTF-8, as the HTML body parser does
///   (`String::from_utf8_lossy`). Non-UTF-8 CSS decoding is future work.
///
fn fetch_external_stylesheets(
    doc: &mut UncascadedDocument,
    options: &ParseOptions<'_>,
    import_budget: &mut ImportBudget,
) {
    let Some(network) = options.network else {
        return;
    };

    // HTML Standard §4.2.7 "The base element" / "document base URL": a
    // <base href> in <head>, if present, overrides options.base_url as the
    // base for resolving <link href> and inline stylesheet imports.
    let effective_base = effective_document_base_url(doc, options.base_url.as_ref());

    // `finish()` has already projected inline sources into this public Vec.
    // Rebuild it in the order of the original head elements so a fetched link
    // does not silently move after every inline style.
    let head_sources = crate::sink::collect_head_stylesheet_sources(&doc.dom);
    let mut inline_sources = std::mem::take(&mut doc.stylesheet_sources).into_iter();
    if head_sources.is_empty() {
        // Preserve the generic `parse_with_sink` contract for a consumer sink
        // that supplies stylesheet_sources without a normal HTML `<head>`.
        doc.stylesheet_sources = inline_sources
            .map(|sheet| {
                expand_inline_sheet(
                    sheet,
                    effective_base.as_ref(),
                    network,
                    &mut doc.warnings,
                    import_budget,
                )
            })
            .collect();
        return;
    }
    let mut ordered_sources = Vec::new();
    for source in head_sources {
        match source {
            crate::sink::HeadStylesheetSource::Inline { .. } => {
                if let Some(sheet) = inline_sources.next() {
                    ordered_sources.push(expand_inline_sheet(
                        sheet,
                        effective_base.as_ref(),
                        network,
                        &mut doc.warnings,
                        import_budget,
                    ));
                }
            }
            crate::sink::HeadStylesheetSource::External { node_id, href } => {
                if let Some(sheet) = fetch_link_stylesheet(
                    doc,
                    network,
                    effective_base.as_ref(),
                    node_id,
                    &href,
                    import_budget,
                ) {
                    ordered_sources.push(sheet);
                }
            }
        }
    }
    // Defensive: preserve any inline projection that did not have a matching
    // collector entry if the sink projection changes in the future.
    ordered_sources.extend(inline_sources.map(|sheet| {
        expand_inline_sheet(
            sheet,
            effective_base.as_ref(),
            network,
            &mut doc.warnings,
            import_budget,
        )
    }));
    doc.stylesheet_sources = ordered_sources;
}

/// The stylesheet a `<link rel=stylesheet href>` at `node_id` contributes,
/// fetched from `href` resolved against `base_url` and expanded for its
/// `@import`s. `None` when there is no request to make, the response is
/// empty, or the fetch fails (which records a warning in `doc`).
fn fetch_link_stylesheet(
    doc: &mut UncascadedDocument,
    network: &dyn raikiri_traits::NetworkProvider,
    base_url: Option<&Url>,
    node_id: raikiri_traits::NodeId,
    href: &str,
    import_budget: &mut ImportBudget,
) -> Option<StylesheetSource> {
    // A relative URL without a base, or otherwise invalid href: there is no
    // request to report and the link contributes no stylesheet source.
    let url = resolve_url(href, base_url)?;
    match network.fetch(get_request(
        url.clone(),
        ResourceKind::ExternalStylesheet,
        None,
    )) {
        Ok(fetched) => {
            // A successful empty response contributes no source.
            let css = String::from_utf8_lossy(&fetched.bytes).into_owned();
            (!css.is_empty()).then(|| {
                let expanded = expand_stylesheet_imports_with_budget(
                    &css,
                    Some(&fetched.final_url),
                    Some(&fetched.final_url),
                    Some(network),
                    &mut doc.warnings,
                    import_budget,
                );
                let media = crate::sink::stylesheet_media_attribute(&doc.dom, node_id);
                StylesheetSource {
                    parts: expanded,
                    media,
                }
            })
        }
        Err(error) => {
            doc.warnings.push(link_fetch_warning(node_id, &url, error));
            None
        }
    }
}

/// The warning for a `<link rel=stylesheet>` at `node_id` whose fetch of
/// `url` failed with `error`. A fetch failure is never fatal.
fn link_fetch_warning(
    node_id: raikiri_traits::NodeId,
    url: &Url,
    error: NetworkError,
) -> RenderWarning {
    match error {
        NetworkError::PolicyViolation(violation) => {
            // ResourcePolicy / RenderResources may deny a response because it
            // exceeds a byte cap. Preserve that diagnostic separately from
            // ordinary policy denials.
            RenderWarning::new(
                policy_violation_warning_kind(&violation, violation.kind),
                "<link rel=stylesheet>: fetch denied by resource policy or resource limit"
                    .to_owned(),
            )
            .node_id(Some(node_id))
        }
        error => {
            // Keep a safe, structured summary instead of formatting arbitrary
            // provider-controlled error text into the warning.
            let safe_url = redacted_url(url);
            let summary = network_error_summary(&error);
            RenderWarning::new(
                WarningKind::NetworkFallback {
                    url: safe_url.clone(),
                },
                format!("<link rel=stylesheet>: fetch failed for {safe_url}: {summary}"),
            )
            .node_id(Some(node_id))
        }
    }
}

fn expand_inline_sheet(
    sheet: StylesheetSource,
    base_url: Option<&Url>,
    network: &dyn raikiri_traits::NetworkProvider,
    warnings: &mut Vec<RenderWarning>,
    budget: &mut ImportBudget,
) -> StylesheetSource {
    let parts = sheet
        .parts
        .into_iter()
        .flat_map(|part| {
            let mut expanded = expand_stylesheet_imports_with_budget(
                &part.source,
                base_url,
                None,
                Some(network),
                warnings,
                budget,
            );
            for child in &mut expanded {
                child.media.extend(part.media.iter().cloned());
            }
            expanded
        })
        .collect();
    StylesheetSource {
        parts,
        media: sheet.media,
    }
}

/// Resolve the document's effective base URL for stylesheet, font, and replaced-resource fetches.
///
/// A valid `<base>` in the document is resolved against the caller-provided
/// fallback. Invalid, `data:`, and `javascript:` base URLs fall back to the
/// caller-provided URL, matching the existing link-fetch behavior.
pub fn effective_document_base_url(
    doc: &UncascadedDocument,
    fallback: Option<&Url>,
) -> Option<Url> {
    match crate::sink::find_document_base_href(&doc.dom) {
        Some(base_href) => resolve_url(&base_href, fallback)
            .filter(|url| !matches!(url.scheme(), "data" | "javascript"))
            .or_else(|| fallback.cloned()),
        None => fallback.cloned(),
    }
}

#[cfg(test)]
mod tests;
