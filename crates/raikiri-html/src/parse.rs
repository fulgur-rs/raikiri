//! Public parse entrypoints.

use std::borrow::Cow;
use std::io::Read;

use html5ever::driver::{ParseOpts, parse_document, parse_fragment as parse_html_fragment};
use html5ever::interface::QualName;
use html5ever::tendril::TendrilSink;
use html5ever::tree_builder::TreeSink;
use markup5ever::{LocalName, Namespace};
use raikiri_traits::{
    Body, Method, NetworkError, ParseError, RenderWarning, Request, ResourceKind, StylesheetKind,
    ViolationType, WarningKind,
};
use url::Url;

use crate::import::{
    ImportBudget, expand_stylesheet_imports_with_budget, network_error_summary, redacted_url,
    sanitize_policy_violation,
};
use crate::sink::RaikiriTreeSink;
use crate::types::{ParseOptions, UncascadedDocument};

/// Parse HTML and return an [`UncascadedDocument`] containing the pre-cascade
/// DOM, extracted inline `<style>` elements, and parse warnings.
///
/// Treat `input` as a UTF-8 byte stream. Read failures return
/// [`ParseError::Io`]; input that is not valid UTF-8 returns [`ParseError::Encoding`].
/// This is the current scope; encoding_rs support may be added later.
///
/// # Example
///
/// ```
/// use raikiri_html::{parse, ParseOptions};
/// use raikiri_traits::Dom;
///
/// let html = b"<html><body>Hi</body></html>";
/// let opts = ParseOptions { extra_stylesheets: &[], network: None, base_url: None };
/// let doc = parse(&html[..], &opts).unwrap();
/// // Parse succeeded; dom has root
/// assert_eq!(doc.dom.root_id().0, 0);
/// ```
pub fn parse<R: Read>(
    input: R,
    options: &ParseOptions<'_>,
) -> Result<UncascadedDocument, ParseError> {
    parse_with_sink(input, RaikiriTreeSink::default(), options)
}

/// Parse through a consumer-supplied sink. Consumer wrappers must declare
/// `type Output = UncascadedDocument` and propagate the inner sink's
/// `finish(self)` result.
///
/// After parsing, inject the default UA CSS and `options.extra_stylesheets`
/// into the Document with [`raikiri_dom::Document::add_stylesheet`]
/// (UserAgent and User kinds, respectively). Leading `@import` rules in extra
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
    let buf = read_utf8(input)?;
    let doc = parse_document(sink, ParseOpts::default()).one(buf.as_str());
    finish_document(doc, options)
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
    let buf = read_utf8(input)?;
    let context_name = QualName::new(
        None,
        Namespace::from(context_namespace),
        LocalName::from(context_local_name),
    );
    let doc = parse_html_fragment(
        RaikiriTreeSink::default(),
        ParseOpts::default(),
        context_name,
        Vec::new(),
        context_element_allows_scripting,
    )
    .one(buf.as_str());
    let mut doc = finish_document(doc, options)?;
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

fn read_utf8(mut input: impl Read) -> Result<String, ParseError> {
    // Separate I/O and UTF-8 failures so a reader's InvalidData is not
    // confused with an encoding failure.
    let mut bytes = Vec::new();
    input.read_to_end(&mut bytes).map_err(ParseError::Io)?;
    String::from_utf8(bytes).map_err(|error| ParseError::Encoding {
        label: String::from("utf-8"),
        reason: error.to_string(),
    })
}

fn finish_document(
    mut doc: UncascadedDocument,
    options: &ParseOptions<'_>,
) -> Result<UncascadedDocument, ParseError> {
    // Inject the default UA CSS into the Document.
    doc.dom.add_stylesheet(
        Cow::Borrowed(crate::ua::MINIMAL_UA_CSS),
        StylesheetKind::UserAgent,
    );

    // The HTML document base URL also resolves relative `@import` rules in
    // inline and extra stylesheets. External stylesheets instead use their
    // fetched `FetchedResource::final_url` as the base.
    let effective_base = effective_document_base_url(&doc, options.base_url.as_ref());
    // Share import limits across every stylesheet root in this document. A
    // separate per-root expander would let many inline/link sheets multiply
    // the fetch and expansion caps.
    let mut import_budget = ImportBudget::default();

    // Add consumer-provided extra_stylesheets with User origin
    // (the path that consumes ParseOptions::extra_stylesheets).
    // These were moved from StylesheetKind::Author to StylesheetKind::User
    // so they are not confused with real author-origin stylesheets
    // (such as `<link rel=stylesheet>`) arriving through other paths.
    for extra in options.extra_stylesheets {
        let expanded = expand_stylesheet_imports_with_budget(
            extra,
            effective_base.as_ref(),
            None,
            options.network,
            &mut doc.warnings,
            &mut import_budget,
        );
        doc.dom
            .add_stylesheet(Cow::Owned(expanded), StylesheetKind::User);
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
/// inline `<style>` elements outside the head follow. The raikiri umbrella
/// crate's `build_cascaded` consumes the entire Vec as Author, without API changes.
///
/// `sink::collect_head_stylesheet_sources` detects hrefs using only the
/// `Document` after `finish()`, with no side effects. Actual fetching happens
/// outside `TreeSink::finish()`, keeping the sink free of I/O. If
/// `options.network` is `None`, external stylesheets are ignored. Fetch failures
/// are nonfatal: record them in `doc.warnings` and continue parsing.
///
/// # Known scope limits
///
/// - `Document::stylesheets()` (UA CSS / extra stylesheets) is a separate
///   bucket, so these cascade before head stylesheets. Imports in extra
///   stylesheets expand before this post-processing pass.
/// - **`disabled` / `crossorigin` / `integrity`**:
///   See the `sink::collect_external_stylesheet_hrefs` documentation. The
///   `media` attribute is recorded in `doc.stylesheet_media` and evaluated by
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
    let inline_media = std::mem::take(&mut doc.stylesheet_media);
    let mut inline_sources = std::mem::take(&mut doc.stylesheet_sources)
        .into_iter()
        .enumerate()
        .map(|(index, css)| (css, inline_media.get(index).cloned().flatten()));
    if head_sources.is_empty() {
        // Preserve the generic `parse_with_sink` contract for a consumer sink
        // that supplies stylesheet_sources without a normal HTML `<head>`.
        (doc.stylesheet_sources, doc.stylesheet_media) = inline_sources
            .map(|(css, media)| {
                let css = expand_stylesheet_imports_with_budget(
                    &css,
                    effective_base.as_ref(),
                    None,
                    Some(network),
                    &mut doc.warnings,
                    import_budget,
                );
                (css, media)
            })
            .unzip();
        return;
    }
    let mut ordered_sources = Vec::new();
    for source in head_sources {
        match source {
            crate::sink::HeadStylesheetSource::Inline { .. } => {
                if let Some((css, media)) = inline_sources.next() {
                    let expanded = expand_stylesheet_imports_with_budget(
                        &css,
                        effective_base.as_ref(),
                        None,
                        Some(network),
                        &mut doc.warnings,
                        import_budget,
                    );
                    ordered_sources.push((expanded, media));
                }
            }
            crate::sink::HeadStylesheetSource::External { node_id, href } => {
                let Some(url) = resolve_url(&href, effective_base.as_ref()) else {
                    // Relative URL without a base, or otherwise invalid href:
                    // there is no request to report and the link contributes no
                    // stylesheet source.
                    continue;
                };

                let request = Request {
                    url: url.clone(),
                    method: Method::Get,
                    content_type: None,
                    headers: Vec::new(),
                    body: Body::Empty,
                    signal: None,
                    kind: ResourceKind::ExternalStylesheet,
                };

                match network.fetch(request) {
                    Ok(fetched) => {
                        // A successful empty response contributes no source.
                        let css = String::from_utf8_lossy(&fetched.bytes).into_owned();
                        if !css.is_empty() {
                            let expanded = expand_stylesheet_imports_with_budget(
                                &css,
                                Some(&fetched.final_url),
                                Some(&fetched.final_url),
                                Some(network),
                                &mut doc.warnings,
                                import_budget,
                            );
                            let media = crate::sink::stylesheet_media_attribute(&doc.dom, node_id);
                            ordered_sources.push((expanded, media));
                        }
                    }
                    Err(NetworkError::PolicyViolation(violation)) => {
                        // ResourcePolicy / RenderResources may deny a response
                        // because it exceeds a byte cap. Preserve that diagnostic
                        // separately from ordinary policy denials.
                        let kind = match &violation.violation_type {
                            ViolationType::FetchTooLarge { limit, actual }
                            | ViolationType::DecodedTooLarge { limit, actual } => {
                                WarningKind::ResourceLimitExceeded {
                                    kind: violation.kind,
                                    limit: *limit,
                                    actual: *actual,
                                }
                            }
                            _ => WarningKind::PolicyWarning {
                                violation: sanitize_policy_violation((*violation).clone()),
                            },
                        };
                        doc.warnings.push(RenderWarning {
                            kind,
                            node_id: Some(node_id),
                            details: "<link rel=stylesheet>: fetch denied by resource policy or resource limit"
                                .to_owned(),
                        });
                    }
                    Err(err) => {
                        // All other errors are non-fatal. Keep a safe,
                        // structured summary instead of formatting arbitrary
                        // provider-controlled error text into the warning.
                        let safe_url = redacted_url(&url);
                        let summary = network_error_summary(&err);
                        doc.warnings.push(RenderWarning {
                            kind: WarningKind::NetworkFallback {
                                url: safe_url.clone(),
                            },
                            node_id: Some(node_id),
                            details: format!(
                                "<link rel=stylesheet>: fetch failed for {safe_url}: {summary}"
                            ),
                        });
                    }
                }
            }
        }
    }
    // Defensive: preserve any inline projection that did not have a matching
    // collector entry if the sink projection changes in the future.
    ordered_sources.extend(inline_sources.map(|(css, media)| {
        let css = expand_stylesheet_imports_with_budget(
            &css,
            effective_base.as_ref(),
            None,
            Some(network),
            &mut doc.warnings,
            import_budget,
        );
        (css, media)
    }));
    (doc.stylesheet_sources, doc.stylesheet_media) = ordered_sources.into_iter().unzip();
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

/// Resolve `href` against `base`. Absolute URLs do not need a base; relative
/// URLs are rejected when no base is available.
fn resolve_url(href: &str, base: Option<&Url>) -> Option<Url> {
    match base {
        Some(base) => base.join(href).ok(),
        None => Url::parse(href).ok(),
    }
}

#[cfg(test)]
mod fragment_root_tests {
    use super::*;

    #[test]
    fn fragment_root_with_zero_or_multiple_children_is_not_flattened() {
        let mut empty = raikiri_dom::Document::new();
        flatten_fragment_root(&mut empty);
        assert!(
            empty
                .get_node(empty.root_index())
                .unwrap()
                .children
                .is_empty()
        );

        let mut multiple = raikiri_dom::Document::new();
        let root = multiple.root_index();
        let first = multiple.append_element(Some(root), "div", Default::default(), None::<&str>);
        let second = multiple.append_element(Some(root), "span", Default::default(), None::<&str>);
        flatten_fragment_root(&mut multiple);
        assert_eq!(
            multiple.get_node(root).unwrap().children,
            vec![first, second]
        );
    }

    #[test]
    fn fragment_root_with_non_html_or_foreign_html_child_is_not_flattened() {
        let mut non_html = raikiri_dom::Document::new();
        let root = non_html.root_index();
        let div = non_html.append_element(Some(root), "div", Default::default(), None::<&str>);
        flatten_fragment_root(&mut non_html);
        assert_eq!(non_html.get_node(root).unwrap().children, vec![div]);

        let mut foreign_html = raikiri_dom::Document::new();
        let root = foreign_html.root_index();
        let html =
            foreign_html.append_element(Some(root), "html", Default::default(), None::<&str>);
        foreign_html.set_element_namespace(html, Some("urn:foreign".into()));
        let child = foreign_html.append_element(Some(html), "g", Default::default(), None::<&str>);
        flatten_fragment_root(&mut foreign_html);
        assert_eq!(foreign_html.get_node(root).unwrap().children, vec![html]);
        assert_eq!(foreign_html.get_node(html).unwrap().children, vec![child]);
    }
}
