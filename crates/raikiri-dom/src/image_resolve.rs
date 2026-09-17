//! Pre-layout `<img>` intrinsic-size resolution.
//!
//! Mirrors [`crate::layout::preshape_text`]'s role: runs once before taffy
//! layout, writes results onto [`crate::node::Node`], so the taffy
//! leaf-measure closures (`crate::taffy_impl`) stay synchronous flat reads
//! with no new plumbing into taffy's own trait surface.
//!
//! Unlike `preshape_text` (which clears every node's prior `text_layout`
//! before re-shaping, see its Step 0 in
//! [`crate::layout::layout_single_page`]), this pass does not clear a
//! previous run's `image_intrinsic_size` before writing new results. Every
//! `<img>` node visited here is overwritten with the current resolve
//! outcome (`Some` or `None`) each call, so re-running on the same
//! `Document` is still correct; the difference only matters if a caller
//! removes/detaches `<img>` nodes between calls without a corresponding
//! `layout_single_page` (non-resolver) pass to reset them.

use raikiri_traits::{ReplacedResolver, ResolverRequest};
use url::Url;

use crate::document::Document;
use crate::node::NodeData;

/// Walks every element in `document`'s arena, resolves `<img src="...">`
/// intrinsic size via `resolver`, and stores the result on each node (see
/// [`crate::node::Node::image_intrinsic_size`]).
///
/// Only absolute `src` URLs are resolved (`Url::parse` must succeed
/// directly) — relative-URL resolution against a document base URL is out
/// of scope (see the design doc's Global Constraints). An `<img>` with an
/// unparsable/relative/missing `src`, or for which `resolver.resolve()`
/// returns `Err`, is left with `image_intrinsic_size = None` (renders as a
/// 0×0 replaced box — no fallback size in this scope).
pub(crate) fn resolve_images(document: &mut Document, resolver: &dyn ReplacedResolver) {
    for node in document.nodes.iter_mut() {
        let NodeData::Element(element) = &mut node.data else {
            continue;
        };
        if element.tag_name.as_str() != "img" {
            continue;
        }
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
        element.image_intrinsic_size = resolver
            .resolve(ResolverRequest::new(&url))
            .ok()
            .map(|resolved| (resolved.intrinsic.width, resolved.intrinsic.height));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use raikiri_traits::{IntrinsicBox, ResolveDisposition, ResolvedIntrinsic, ResolverError};
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

    struct AlwaysErrResolver;
    impl ReplacedResolver for AlwaysErrResolver {
        fn resolve(&self, _req: ResolverRequest<'_>) -> Result<ResolvedIntrinsic, ResolverError> {
            Err(ResolverError::Decode("nope".into()))
        }
    }

    #[test]
    fn resolves_img_with_absolute_src() {
        let mut doc = Document::new();
        let root = doc.root_index();
        let img = doc.append_element(Some(root), "img", Style::default(), None::<&str>);
        doc.set_element_attributes(img, vec![("src".into(), "file:///x.png".into())]);

        resolve_images(&mut doc, &FixedSizeResolver(10.0, 20.0));

        assert_eq!(doc.nodes[img].image_intrinsic_size(), Some((10.0, 20.0)));
    }

    #[test]
    fn skips_img_with_relative_src() {
        let mut doc = Document::new();
        let root = doc.root_index();
        let img = doc.append_element(Some(root), "img", Style::default(), None::<&str>);
        doc.set_element_attributes(img, vec![("src".into(), "relative.png".into())]);

        resolve_images(&mut doc, &FixedSizeResolver(10.0, 20.0));

        assert_eq!(doc.nodes[img].image_intrinsic_size(), None);
    }

    #[test]
    fn leaves_none_on_resolver_error() {
        let mut doc = Document::new();
        let root = doc.root_index();
        let img = doc.append_element(Some(root), "img", Style::default(), None::<&str>);
        doc.set_element_attributes(img, vec![("src".into(), "file:///x.png".into())]);

        resolve_images(&mut doc, &AlwaysErrResolver);

        assert_eq!(doc.nodes[img].image_intrinsic_size(), None);
    }

    #[test]
    fn ignores_non_img_elements() {
        let mut doc = Document::new();
        let root = doc.root_index();
        let div = doc.append_element(Some(root), "div", Style::default(), None::<&str>);
        doc.set_element_attributes(div, vec![("src".into(), "file:///x.png".into())]);

        resolve_images(&mut doc, &FixedSizeResolver(10.0, 20.0));

        assert_eq!(doc.nodes[div].image_intrinsic_size(), None);
    }
}
