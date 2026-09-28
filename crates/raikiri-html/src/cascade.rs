//! Cascade orchestration over a parsed [`UncascadedDocument`].
//!
//! Assembles the UA, user, and author stylesheets associated with a parsed
//! document into a [`RuleTree`] and runs the element and `@page` cascades.

use raikiri_style::{
    CascadeResult, ConsumerPropertyRegistration, MediaContext, Origin, PageContextQuery, RuleTree,
    cascade_with_media_context_for_page,
};
use raikiri_traits::StylesheetKind;

use crate::UncascadedDocument;

/// Take the UA and consumer-supplied stylesheets from the Document, assign
/// origins, build a RuleTree, add inline `<style>` elements collected by
/// raikiri-html during parsing and fetched `<link rel="stylesheet">` sources as
/// Author stylesheets, then run the cascade.
///
/// Currently `raikiri_style::cascade` always returns `Ok`, so we call `expect`
/// internally. Consider exposing its Result if that changes.
///
/// Consumers obtain per-node ComputedValues in two steps:
/// [`crate::parse`](fn@crate::parse) followed by [`build_cascaded`].
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
/// `Document.stylesheets()` (UA plus `extra_stylesheets`, inserted during parsing)
/// enters the RuleTree first. Next, `stylesheet_sources` (inline styles in the
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
pub fn build_cascaded(doc: &UncascadedDocument) -> CascadeResult {
    build_cascaded_with_media_context(doc, &MediaContext::default())
}

/// Build a cascade that retains the supplied consumer-owned properties.
pub fn build_cascaded_with_consumer_properties(
    doc: &UncascadedDocument,
    consumer_properties: &[ConsumerPropertyRegistration],
) -> CascadeResult {
    build_cascaded_with_media_context_for_page_and_consumer_properties(
        doc,
        &MediaContext::default(),
        &PageContextQuery::default(),
        consumer_properties,
    )
}

/// Build the cascade for one page-context query using the default media context.
pub fn build_cascaded_for_page(
    doc: &UncascadedDocument,
    page_query: &PageContextQuery,
) -> CascadeResult {
    build_cascaded_with_media_context_for_page(doc, &MediaContext::default(), page_query)
}

/// Build the rule tree and run the cascade for an explicit media context.
///
/// [`build_cascaded`] remains the compatibility entry point and uses the
/// default paged (`print`) context.
pub fn build_cascaded_with_media_context(
    doc: &UncascadedDocument,
    media_context: &MediaContext,
) -> CascadeResult {
    build_cascaded_with_media_context_for_page(doc, media_context, &PageContextQuery::default())
}

/// Build the element and `@page` cascades for one page-context query.
///
/// The first-page render path uses this entry point with `is_first` and
/// `is_right` set. A future page-stream driver can call it once per page with
/// the page name and pseudo-page state selected by its break algorithm.
pub fn build_cascaded_with_media_context_for_page(
    doc: &UncascadedDocument,
    media_context: &MediaContext,
    page_query: &PageContextQuery,
) -> CascadeResult {
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
pub fn build_cascaded_with_media_context_for_page_and_consumer_properties(
    doc: &UncascadedDocument,
    media_context: &MediaContext,
    page_query: &PageContextQuery,
    consumer_properties: &[ConsumerPropertyRegistration],
) -> CascadeResult {
    let tree = build_rule_tree_with_consumer_properties(doc, consumer_properties);
    cascade_with_media_context_for_page(&doc.dom, &tree, media_context, page_query)
        .expect("cascade は常に Ok のはず")
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
    for (source, kind) in doc.dom.stylesheets() {
        let origin = stylesheet_kind_to_origin(kind);
        tree.add_stylesheet(source, origin);
    }

    // Add the head/body inline styles and fetched head links collected through
    // raikiri-html's template-inert filter during parsing as Author stylesheets.
    for source in &doc.stylesheet_sources {
        tree.add_stylesheet(source, Origin::Author);
    }

    tree
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
mod smoke_tests {
    use super::*;

    #[test]
    fn stylesheet_kind_to_origin_matches_spec() {
        assert_eq!(
            stylesheet_kind_to_origin(StylesheetKind::UserAgent),
            Origin::UserAgent,
        );
        assert_eq!(
            stylesheet_kind_to_origin(StylesheetKind::User),
            Origin::User,
        );
        assert_eq!(
            stylesheet_kind_to_origin(StylesheetKind::Author),
            Origin::Author,
        );
    }
}
