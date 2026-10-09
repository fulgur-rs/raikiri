//! Cascade orchestration over a parsed [`UncascadedDocument`].
//!
//! Assembles the UA, user, and author stylesheets associated with a parsed
//! document into a [`RuleTree`] and runs the element and `@page` cascades.

use raikiri_style::{
    CascadeError, CascadeOptions, CascadeResult, ConsumerPropertyRegistration, MediaContext,
    Origin, PageContextQuery, RuleTree, cascade_with_options,
};
use raikiri_traits::StylesheetKind;

use crate::UncascadedDocument;

/// Take the UA and consumer-supplied stylesheets from the Document, assign
/// origins, build a RuleTree, add inline `<style>` elements collected by
/// raikiri-html during parsing and fetched `<link rel="stylesheet">` sources as
/// Author stylesheets, then run the cascade within the default
/// [`raikiri_style::CascadeLimits`].
///
/// Consumers obtain per-node ComputedValues in two steps:
/// [`crate::parse`](fn@crate::parse) followed by [`build_cascaded`].
///
/// # Errors
///
/// Fails when the document and its stylesheets pass a cascade limit, the
/// allocator refuses the cascade's result, or the stylesheets have more
/// rules, selectors or declarations than the cascade can number; see
/// [`raikiri_style::cascade_with_options`].
///
/// # Collection scope for DOM `<style>` elements
///
/// Consume `UncascadedDocument::stylesheet_sources` as Author stylesheets.
/// During [`crate::parse`](fn@crate::parse), `extract_inline_stylesheets` first
/// collects stylesheet-bearing head elements in their original order, then
/// appends inline `<style>` elements outside the head in document order.
/// HTML/XHTML and SVG `<style>` elements are included; MathML `<style>` is not.
/// `<template>` subtrees have already been skipped per spec §14.1 inertness.
///
/// # DOM `<style>` (Author) vs `extra_stylesheets` (User)
///
/// `Document.stylesheets()` and `user_stylesheet_sources` enter the RuleTree
/// first. Next, `stylesheet_sources` (inline styles in the
/// head/body and fetched head links) enters as Author. Previously,
/// `extra_stylesheets` was also tagged `Author`, so conflicts with DOM `<style>`
/// were resolved by source-order tie-breaking within one origin
/// (later sources won). Since
/// `extra_stylesheets` was retagged [`Origin::User`], the two sources now have
/// different origins: origin rank determines the winner, regardless of
/// specificity or source order.
///
/// **For normal declarations**, [`Origin::Author`] (rank 3) outranks [`Origin::User`]
/// (rank 1), so **DOM `<style>` overrides `extra_stylesheets`**.
/// This matches the old outcome, which was incidental to same-origin source-order
/// tie-breaking; the reason is now the difference between origin ranks.
///
/// **`!important` can reverse the outcome** (CSS Cascading L4 §6.3 reverses
/// origin order for important declarations). If `extra_stylesheets` declares
/// `!important` ([`Origin::User`] important rank 6), it wins regardless of the
/// DOM `<style>` importance (Author normal rank 3 or important rank 4,
/// both below 6). Conversely, if `extra_stylesheets` is normal (rank 1),
/// DOM `<style>` wins whether normal or important (rank 3 or 4).
/// In practice, only the importance of `extra_stylesheets` decides the outcome
/// between these two sources.
///
/// # Dependency direction
///
/// Translate `StylesheetKind → Origin` here in raikiri-html, which sits above
/// both raikiri-style and raikiri-dom, rather than in either lower crate.
/// This avoids a reverse dependency between the lower crates.
pub fn build_cascaded(doc: &UncascadedDocument) -> Result<CascadeResult, CascadeError> {
    build_cascaded_with_media_context(doc, &MediaContext::default())
}

/// Build a cascade that retains the supplied consumer-owned properties.
///
/// # Errors
///
/// See [`build_cascaded`].
pub fn build_cascaded_with_consumer_properties(
    doc: &UncascadedDocument,
    consumer_properties: &[ConsumerPropertyRegistration],
) -> Result<CascadeResult, CascadeError> {
    build_cascaded_with_media_context_for_page_and_consumer_properties(
        doc,
        &MediaContext::default(),
        &PageContextQuery::default(),
        consumer_properties,
    )
}

/// Build the cascade for one page-context query using the default media context.
///
/// # Errors
///
/// See [`build_cascaded`].
pub fn build_cascaded_for_page(
    doc: &UncascadedDocument,
    page_query: &PageContextQuery,
) -> Result<CascadeResult, CascadeError> {
    build_cascaded_with_media_context_for_page(doc, &MediaContext::default(), page_query)
}

/// Build the rule tree and run the cascade for an explicit media context.
///
/// [`build_cascaded`] remains the compatibility entry point and uses the
/// default paged (`print`) context.
///
/// # Errors
///
/// See [`build_cascaded`].
pub fn build_cascaded_with_media_context(
    doc: &UncascadedDocument,
    media_context: &MediaContext,
) -> Result<CascadeResult, CascadeError> {
    build_cascaded_with_media_context_for_page(doc, media_context, &PageContextQuery::default())
}

/// Build the element and `@page` cascades for one page-context query.
///
/// The first-page render path uses this entry point with `is_first` and
/// `is_right` set. A future page-stream driver can call it once per page with
/// the page name and pseudo-page state selected by its break algorithm.
///
/// # Errors
///
/// See [`build_cascaded`].
pub fn build_cascaded_with_media_context_for_page(
    doc: &UncascadedDocument,
    media_context: &MediaContext,
    page_query: &PageContextQuery,
) -> Result<CascadeResult, CascadeError> {
    build_cascaded_with_media_context_for_page_and_consumer_properties(
        doc,
        media_context,
        page_query,
        &[],
    )
}

/// Build a page-aware cascade while retaining registered consumer properties.
///
/// Registration is optional and has no effect on the compatibility cascade.
/// Registered properties are parsed into the existing inherited custom-property
/// environment, then exposed through the neutral observer at render time.
///
/// # Errors
///
/// See [`build_cascaded`].
pub fn build_cascaded_with_media_context_for_page_and_consumer_properties(
    doc: &UncascadedDocument,
    media_context: &MediaContext,
    page_query: &PageContextQuery,
    consumer_properties: &[ConsumerPropertyRegistration],
) -> Result<CascadeResult, CascadeError> {
    build_cascaded_with_options(
        doc,
        media_context,
        page_query,
        consumer_properties,
        &CascadeOptions::default(),
    )
}

/// Build a page-aware cascade that retains registered consumer properties,
/// within `options.limits`.
///
/// # Errors
///
/// Fails when the document and its stylesheets pass one of `options.limits`,
/// the allocator refuses the cascade's result, or the stylesheets have more
/// rules, selectors or declarations than the cascade can number; see
/// [`raikiri_style::cascade_with_options`].
pub fn build_cascaded_with_options(
    doc: &UncascadedDocument,
    media_context: &MediaContext,
    page_query: &PageContextQuery,
    consumer_properties: &[ConsumerPropertyRegistration],
    options: &CascadeOptions,
) -> Result<CascadeResult, CascadeError> {
    let tree = build_rule_tree_with_consumer_properties(doc, consumer_properties);
    cascade_with_options(&doc.dom, &tree, media_context, page_query, options)
}

/// Build the stylesheet rule tree used by the document cascade.
///
/// Keeping this operation separate lets a paged renderer retain the parsed
/// `@page` rules while it performs a per-page cascade in a later page loop.
pub fn build_rule_tree(doc: &UncascadedDocument) -> RuleTree {
    build_rule_tree_with_consumer_properties(doc, &[])
}

/// Build a rule tree configured for the supplied consumer-owned properties.
pub fn build_rule_tree_with_consumer_properties(
    doc: &UncascadedDocument,
    consumer_properties: &[ConsumerPropertyRegistration],
) -> RuleTree {
    let mut tree = RuleTree::empty_with_consumer_properties(consumer_properties);

    // Map every stylesheet associated with the Document to an Origin by kind.
    // Call order (insertion order) determines cascade source_order.
    let mut dom_stylesheets = doc.dom.stylesheets();
    for (source, kind) in dom_stylesheets
        .by_ref()
        .take(doc.user_stylesheet_insertion_index)
    {
        let origin = stylesheet_kind_to_origin(kind);
        tree.add_stylesheet(source, origin);
    }

    for sheet in &doc.user_stylesheet_sources {
        add_sheet(&mut tree, sheet, Origin::User);
    }
    for (source, kind) in dom_stylesheets {
        let origin = stylesheet_kind_to_origin(kind);
        tree.add_stylesheet(source, origin);
    }

    // Add inline styles and fetched links in their original document order.
    for sheet in &doc.stylesheet_sources {
        add_sheet(&mut tree, sheet, Origin::Author);
    }

    tree
}

fn add_sheet(tree: &mut RuleTree, sheet: &crate::StylesheetSource, origin: Origin) {
    for part in &sheet.parts {
        let media: Vec<&str> = part
            .media
            .iter()
            .map(String::as_str)
            .chain(sheet.media.as_deref())
            .collect();
        tree.add_stylesheet_with_media_conditions(&part.source, origin, &media);
    }
}

/// Translate DOM-level [`StylesheetKind`] (raikiri-traits) into cascade-level
/// [`Origin`] (raikiri-style) here to preserve dependency direction.
///
/// `StylesheetKind` is `#[non_exhaustive]` in another crate, so an exhaustive match
/// is impossible. To prevent silent misrouting if a variant is added later,
/// make the `_` arm fail loudly with `unreachable!` (the three current variants
/// UserAgent, User, and Author are handled explicitly).
fn stylesheet_kind_to_origin(kind: StylesheetKind) -> Origin {
    match kind {
        StylesheetKind::UserAgent => Origin::UserAgent,
        StylesheetKind::User => Origin::User,
        StylesheetKind::Author => Origin::Author,
        // `StylesheetKind` is `#[non_exhaustive]`. The UserAgent, User, and Author
        // variants are all handled above. If a future variant is not mapped here,
        // execution reaches this arm; panic rather than silently assigning a wrong
        // origin so developers notice that this mapping must be updated.
        // cov:ignore: defensive `_` arm for a cross-crate `#[non_exhaustive]` enum —
        // unreachable by construction while all 3 current variants are matched above;
        // only becomes reachable if a future variant is added upstream without a
        // corresponding arm here (the panic message tells the dev to add one).
        _ => unreachable!(
            "StylesheetKind variant not yet mapped to Origin — update stylesheet_kind_to_origin in raikiri-html crate"
        ),
    }
}

/// Smoke tests in tests/build_cascaded.rs confirm that cascade orchestration
/// uses Document `<style>` sources as Author, without counting UA CSS from
/// Document.stylesheets a second time.
#[cfg(test)]
mod tests;
