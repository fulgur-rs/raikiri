//! Pre-layout `<img>` intrinsic-size resolution.
//!
//! Mirrors [`crate::layout::preshape_text`]'s role: runs once before taffy
//! layout, writes results onto [`crate::node::Node`], so the taffy
//! leaf-measure closures (`crate::taffy_impl`) stay synchronous flat reads
//! with no new plumbing into taffy's own trait surface. Like
//! `preshape_text`'s Step-0 `text_layout` clear (see
//! [`crate::layout::layout_single_page`]), every `<img>` node this pass
//! visits has `image_intrinsic_size` reset to `None` before being
//! re-resolved, so re-running on the same `Document` after a mutation (e.g.
//! `src` changed or removed) never observes a stale value from a previous
//! pass.
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
        // Reset before the src lookup so every early `continue` below
        // (missing/unparsable/relative src) leaves this at `None` rather
        // than a previous pass's stale value.
        element.image_intrinsic_size = None;
        let Some(src) = element
            .attributes
            .iter()
            .find(|a| a.local.as_str() == "src")
            .map(|a| a.value.as_str())
        else {
            continue;
        };
        let Ok(url) = Url::parse(src) else {
            continue;
        };
        let resolved = resolver.resolve(ResolverRequest::new(&url))?;
        element.image_intrinsic_size = Some((resolved.intrinsic.width, resolved.intrinsic.height));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use raikiri_traits::{IntrinsicBox, ResolveDisposition, ResolvedIntrinsic};
    use std::cell::Cell;
    use taffy::Style;

    struct FixedSizeResolver(f32, f32);
    impl ReplacedResolver for FixedSizeResolver {
        fn resolve(&self, _req: ResolverRequest<'_>) -> Result<ResolvedIntrinsic, ResolverError> {
            Ok(ResolvedIntrinsic {
                intrinsic: IntrinsicBox::new(self.0, self.1),
                disposition: ResolveDisposition::Ok,
            })
        }
    }

    /// Returns an intentional Consumer-side fallback size. Per
    /// `ReplacedResolver`'s contract this is a *success*, not an error — the
    /// disposition is only a reporting channel, and the `intrinsic` it
    /// carries is as usable as `ResolveDisposition::Ok`'s.
    struct FallbackResolver(f32, f32);
    impl ReplacedResolver for FallbackResolver {
        fn resolve(&self, _req: ResolverRequest<'_>) -> Result<ResolvedIntrinsic, ResolverError> {
            Ok(ResolvedIntrinsic {
                intrinsic: IntrinsicBox::new(self.0, self.1),
                disposition: ResolveDisposition::Fallback {
                    reason: "placeholder".into(),
                },
            })
        }
    }

    struct AlwaysErrResolver;
    impl ReplacedResolver for AlwaysErrResolver {
        fn resolve(&self, _req: ResolverRequest<'_>) -> Result<ResolvedIntrinsic, ResolverError> {
            Err(ResolverError::Decode("nope".into()))
        }
    }

    /// Counts calls and fails every one, so a test can pin *how far* the
    /// walk got before the terminal error stopped it.
    #[derive(Default)]
    struct CountingErrResolver {
        calls: Cell<usize>,
    }
    impl ReplacedResolver for CountingErrResolver {
        fn resolve(&self, _req: ResolverRequest<'_>) -> Result<ResolvedIntrinsic, ResolverError> {
            self.calls.set(self.calls.get() + 1);
            Err(ResolverError::Decode("nope".into()))
        }
    }

    /// Fails loudly if resolved at all — for nodes this pass must never
    /// reach (resolving them would mean a real `NetworkProvider::fetch`
    /// side effect for an element that is never laid out or painted).
    struct NeverCalledResolver;
    impl ReplacedResolver for NeverCalledResolver {
        fn resolve(&self, req: ResolverRequest<'_>) -> Result<ResolvedIntrinsic, ResolverError> {
            unimplemented!(
                "resolve() must not be called for an <img> outside the rendered \
                 flat tree (was called for {})",
                req.url()
            )
        }
    }

    #[test]
    fn skips_img_inside_template_contents() {
        // An `<img>` under a `<template>` is never laid out or painted, so
        // resolving it would perform a real fetch for nothing. Membership is
        // decided by `Node::is_in_document()` (the repo-wide convention — see
        // this crate's module doc "Flat tree membership"), not by a tag-name
        // check for "template".
        let mut doc = Document::new();
        let root = doc.root_index();
        let tmpl = doc.append_element(Some(root), "template", Style::default(), None::<&str>);
        let img = doc.append_element(Some(tmpl), "img", Style::default(), None::<&str>);
        doc.set_element_attributes(img, vec![("src".into(), "file:///x.png".into())]);
        doc.mark_in_document_flags();
        assert!(
            !doc.nodes[img].is_in_document(),
            "test premise: a <template> descendant must be !is_in_document"
        );

        resolve_images(&mut doc, &NeverCalledResolver)
            .expect("an inert <img> must be skipped, not resolved");

        assert_eq!(doc.nodes[img].image_intrinsic_size(), None);
    }

    #[test]
    fn resolves_img_with_absolute_src() {
        let mut doc = Document::new();
        let root = doc.root_index();
        let img = doc.append_element(Some(root), "img", Style::default(), None::<&str>);
        doc.set_element_attributes(img, vec![("src".into(), "file:///x.png".into())]);

        resolve_images(&mut doc, &FixedSizeResolver(10.0, 20.0)).expect("resolve Ok");

        assert_eq!(doc.nodes[img].image_intrinsic_size(), Some((10.0, 20.0)));
    }

    /// A `Fallback` disposition is a successful resolve: the pass must use
    /// its `intrinsic` just like `Ok`'s, and must not treat it as an error.
    #[test]
    fn uses_intrinsic_from_a_fallback_disposition() {
        let mut doc = Document::new();
        let root = doc.root_index();
        let img = doc.append_element(Some(root), "img", Style::default(), None::<&str>);
        doc.set_element_attributes(img, vec![("src".into(), "file:///x.png".into())]);

        resolve_images(&mut doc, &FallbackResolver(30.0, 40.0))
            .expect("a Fallback disposition is Ok, not an error");

        assert_eq!(doc.nodes[img].image_intrinsic_size(), Some((30.0, 40.0)));
    }

    #[test]
    fn skips_img_with_relative_src() {
        let mut doc = Document::new();
        let root = doc.root_index();
        let img = doc.append_element(Some(root), "img", Style::default(), None::<&str>);
        doc.set_element_attributes(img, vec![("src".into(), "relative.png".into())]);

        resolve_images(&mut doc, &FixedSizeResolver(10.0, 20.0)).expect("resolve Ok");

        assert_eq!(doc.nodes[img].image_intrinsic_size(), None);
    }

    /// A resolver `Err` is terminal by `ReplacedResolver`'s contract, so this
    /// pass propagates it instead of leaving the element unsized. Swallowing
    /// it would silently perform the Consumer's fallback on its behalf —
    /// which the Consumer expresses as `ResolveDisposition::Fallback`.
    #[test]
    fn propagates_resolver_error() {
        let mut doc = Document::new();
        let root = doc.root_index();
        let img = doc.append_element(Some(root), "img", Style::default(), None::<&str>);
        doc.set_element_attributes(img, vec![("src".into(), "file:///x.png".into())]);

        let err = resolve_images(&mut doc, &AlwaysErrResolver)
            .expect_err("a resolver Err must propagate, not be swallowed");

        assert!(
            matches!(err, ResolverError::Decode(_)),
            "the resolver's own error must come back verbatim, got {err:?}"
        );
    }

    /// "Terminal" means the walk stops: a second `<img>` after the failing
    /// one is never resolved (and so never fetched).
    #[test]
    fn resolver_error_stops_the_walk_at_the_first_failure() {
        let mut doc = Document::new();
        let root = doc.root_index();
        let first = doc.append_element(Some(root), "img", Style::default(), None::<&str>);
        doc.set_element_attributes(first, vec![("src".into(), "file:///a.png".into())]);
        let second = doc.append_element(Some(root), "img", Style::default(), None::<&str>);
        doc.set_element_attributes(second, vec![("src".into(), "file:///b.png".into())]);

        let resolver = CountingErrResolver::default();
        resolve_images(&mut doc, &resolver).expect_err("first <img> fails");

        assert_eq!(
            resolver.calls.get(),
            1,
            "the walk must return on the first failure, not keep resolving"
        );
    }

    #[test]
    fn ignores_non_img_elements() {
        let mut doc = Document::new();
        let root = doc.root_index();
        let div = doc.append_element(Some(root), "div", Style::default(), None::<&str>);
        doc.set_element_attributes(div, vec![("src".into(), "file:///x.png".into())]);

        resolve_images(&mut doc, &FixedSizeResolver(10.0, 20.0)).expect("resolve Ok");

        assert_eq!(doc.nodes[div].image_intrinsic_size(), None);
    }

    #[test]
    fn re_resolving_after_src_becomes_unresolvable_clears_stale_value() {
        // Regression pin for the re-entrance reset: a node that previously
        // resolved to `Some` must not keep that value once its `src` is
        // mutated to something unresolvable and `resolve_images` runs again
        // (mirrors `preshape_text`'s Step-0 `text_layout = None` clear).
        let mut doc = Document::new();
        let root = doc.root_index();
        let img = doc.append_element(Some(root), "img", Style::default(), None::<&str>);
        doc.set_element_attributes(img, vec![("src".into(), "file:///x.png".into())]);
        resolve_images(&mut doc, &FixedSizeResolver(10.0, 20.0)).expect("resolve Ok");
        assert_eq!(doc.nodes[img].image_intrinsic_size(), Some((10.0, 20.0)));

        doc.set_element_attributes(img, vec![("src".into(), "relative.png".into())]);
        resolve_images(&mut doc, &FixedSizeResolver(10.0, 20.0)).expect("resolve Ok");

        assert_eq!(doc.nodes[img].image_intrinsic_size(), None);
    }
}
