//! Pre-layout `<img>` intrinsic-size resolution.
//!
//! Mirrors [`crate::layout`]'s `preshape_text` role: runs once before taffy
//! layout, writes results onto [`crate::node::Node`], so the taffy
//! leaf-measure closures (`crate::taffy_impl`) stay synchronous flat reads
//! with no new integration logic into taffy's own trait surface. Each pass
//! reconciles the current result, including clearing stale sizes for removed
//! or invalid `src` values. A changed intrinsic size marks layout caches dirty
//! even when the caller reuses the same cascade generation.
//!
//! Two kinds of node are *not* visited, and so keep whatever value they
//! already held: nodes outside the rendered flat tree (`<template>`
//! descendants and other inert subtrees), which are skipped before the
//! reset; and — once a resolve fails — every node after the failing one,
//! since a resolver `Err` is terminal and returns immediately.

use raikiri_traits::{ReplacedResolver, ResolverError, ResolverRequest};
use url::Url;

use crate::document::Document;
use crate::node::NodeData;

/// Walks every element in `document`'s arena, resolves `<img src="...">`
/// intrinsic size via `resolver`, and stores the result on each node (see
/// [`crate::node::Node::image_intrinsic_size`]).
///
/// Only nodes that are part of the rendered flat tree are visited —
/// [`crate::node::Node::is_in_document`] gates the walk, so an `<img>`
/// inside a `<template>` (or any other inert subtree) never reaches
/// `resolver.resolve()`, and therefore never triggers the fetch behind it,
/// for an element that is never laid out or painted. Membership is decided
/// by that flag rather than by a tag-name test, per this crate's module
/// doc "Flat tree membership". The caller owns flag freshness
/// ([`Document::mark_in_document_flags`]);
/// [`crate::layout::layout_single_page_with_resolver`] refreshes them
/// immediately before calling here.
///
/// Only absolute `src` URLs are resolved (`Url::parse` must succeed
/// directly). Relative-URL resolution against a document base URL is out
/// of scope, because no base URL is persisted on [`Document`] past parse
/// time. An `<img>` with an unparsable/relative/missing `src` is left with
/// `image_intrinsic_size = None` (renders as a 0×0 replaced box — no
/// fallback size in this scope).
///
/// # Errors
/// [`ResolverError`] — propagated verbatim from the first
/// `resolver.resolve()` that fails, which also ends the walk. A resolver
/// `Err` is terminal by contract (see
/// [`raikiri_traits::ReplacedResolver`]): graceful degradation is the
/// Consumer's to express, as `Ok(ResolvedIntrinsic { disposition:
/// ResolveDisposition::Fallback { .. }, .. })`. This pass therefore treats
/// a `Fallback` disposition exactly like `Ok` — it still uses the returned
/// `intrinsic` — and never substitutes a fallback of its own by swallowing
/// an `Err`.
#[allow(clippy::result_large_err)]
pub(crate) fn resolve_images(
    document: &mut Document,
    resolver: &dyn ReplacedResolver,
) -> Result<(), ResolverError> {
    resolve_images_with_base(document, resolver, None)
}

/// Resolve replaced-element URLs against the document base when they are not
/// already absolute.
#[allow(clippy::result_large_err)]
pub(crate) fn resolve_images_with_base(
    document: &mut Document,
    resolver: &dyn ReplacedResolver,
    base_url: Option<&Url>,
) -> Result<(), ResolverError> {
    let mut intrinsic_size_changed = false;
    let result = (|| {
        for node in document.nodes.iter_mut() {
            if !node.is_in_document() {
                continue;
            }
            let NodeData::Element(element) = &mut node.data else {
                continue;
            };
            if element.tag_name.as_str() != "img" {
                continue;
            }
            let previous_size = element.image_intrinsic_size;
            let src = element
                .attributes
                .iter()
                .find(|attribute| attribute.local.as_str() == "src")
                .map(|attribute| attribute.value.as_str());
            let next_size = if let Some(src) = src {
                let url = Url::parse(src)
                    .ok()
                    .or_else(|| base_url.and_then(|base| base.join(src).ok()));
                if let Some(url) = url {
                    match resolver.resolve(ResolverRequest::new(&url)) {
                        Ok(resolved) => Some((resolved.intrinsic.width, resolved.intrinsic.height)),
                        Err(error) => {
                            if previous_size.is_some() {
                                element.image_intrinsic_size = None;
                                intrinsic_size_changed = true;
                            }
                            return Err(error);
                        }
                    }
                } else {
                    None
                }
            } else {
                None
            };
            if previous_size != next_size {
                element.image_intrinsic_size = next_size;
                intrinsic_size_changed = true;
            }
        }
        Ok(())
    })();
    if intrinsic_size_changed {
        document.invalidate_layout_cache();
    }
    result
}

#[cfg(test)]
mod tests;
