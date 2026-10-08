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

/// Counts calls and fails every one, so a test can check *how far* the
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
    // An `<img>` in a `<template>` element's detached contents fragment is
    // never laid out or painted, so resolving it would perform a real fetch
    // for nothing. Membership is decided by `Node::is_in_document()` (the
    // repo-wide convention — see this crate's module doc "Flat tree
    // membership"), not by a tag-name check for "template". The `<img>` is
    // therefore placed in the contents fragment (the parser's shape), not as
    // an ordinary light-DOM child of the template element (which stays
    // in-document — see `resolves_img_appended_directly_under_template`).
    let mut doc = Document::new();
    let root = doc.root_index();
    let tmpl = doc.append_element(Some(root), "template", Style::default(), None::<&str>);
    let frag = doc.allocate_template_fragment_root(tmpl);
    let img = doc.append_element(Some(frag), "img", Style::default(), None::<&str>);
    doc.set_element_attributes(img, vec![("src".into(), "file:///x.png".into())]);
    doc.mark_in_document_flags();
    assert!(
        !doc.nodes[img].is_in_document(),
        "test premise: a <template> contents-fragment descendant must be !is_in_document"
    );

    resolve_images(&mut doc, &NeverCalledResolver)
        .expect("an inert <img> must be skipped, not resolved");

    assert_eq!(doc.nodes[img].image_intrinsic_size(), None);
}

#[test]
fn resolves_img_appended_directly_under_template() {
    // An `<img>` appended directly under a `<template>` element (the DOM
    // `appendChild` shape) is an ordinary light-DOM child: it stays
    // in-document and is resolved like any other reachable `<img>`.
    let mut doc = Document::new();
    let root = doc.root_index();
    let tmpl = doc.append_element(Some(root), "template", Style::default(), None::<&str>);
    let img = doc.append_element(Some(tmpl), "img", Style::default(), None::<&str>);
    doc.set_element_attributes(img, vec![("src".into(), "file:///x.png".into())]);
    doc.mark_in_document_flags();
    assert!(
        doc.nodes[img].is_in_document(),
        "test premise: a real <template> child must be is_in_document"
    );

    resolve_images(&mut doc, &FixedSizeResolver(10.0, 20.0)).expect("resolve Ok");

    assert_eq!(doc.nodes[img].image_intrinsic_size(), Some((10.0, 20.0)));
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
fn skips_img_with_relative_src_without_a_base() {
    let mut doc = Document::new();
    let root = doc.root_index();
    let img = doc.append_element(Some(root), "img", Style::default(), None::<&str>);
    doc.set_element_attributes(img, vec![("src".into(), "relative.png".into())]);

    resolve_images(&mut doc, &FixedSizeResolver(10.0, 20.0)).expect("resolve Ok");

    assert_eq!(doc.nodes[img].image_intrinsic_size(), None);
}

#[test]
fn resolves_relative_img_src_against_document_base() {
    struct Capture(std::cell::RefCell<Option<Url>>);
    impl ReplacedResolver for Capture {
        fn resolve(&self, req: ResolverRequest<'_>) -> Result<ResolvedIntrinsic, ResolverError> {
            *self.0.borrow_mut() = Some(req.url().clone());
            Ok(ResolvedIntrinsic {
                intrinsic: IntrinsicBox::new(10.0, 20.0),
                disposition: ResolveDisposition::Ok,
            })
        }
    }

    let mut doc = Document::new();
    let root = doc.root_index();
    let img = doc.append_element(Some(root), "img", Style::default(), None::<&str>);
    doc.set_element_attributes(img, vec![("src".into(), "../image.png".into())]);
    let resolver = Capture(std::cell::RefCell::new(None));
    let base = Url::parse("https://example.test/book/chapter.html").unwrap();

    resolve_images_with_base(&mut doc, &resolver, Some(&base)).expect("resolve Ok");

    assert_eq!(
        resolver.0.borrow().as_ref(),
        Some(&Url::parse("https://example.test/image.png").unwrap()),
    );
    assert_eq!(doc.nodes[img].image_intrinsic_size(), Some((10.0, 20.0)));
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

#[test]
fn resolver_error_clears_stale_size_and_dirties_layout() {
    let mut doc = Document::new();
    let root = doc.root_index();
    let img = doc.append_element(Some(root), "img", Style::default(), None::<&str>);
    doc.set_element_attributes(img, vec![("src".into(), "file:///x.png".into())]);
    doc.mark_in_document_flags();

    resolve_images(&mut doc, &FixedSizeResolver(10.0, 20.0)).unwrap();
    doc.layout_dirty = false;
    assert!(resolve_images(&mut doc, &AlwaysErrResolver).is_err());

    assert_eq!(doc.nodes[img].image_intrinsic_size(), None);
    assert!(doc.layout_dirty);
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
    // Regression check for the re-entrance reset: a node that previously
    // resolved to `Some` must not keep that value once its `src` is
    // mutated to something unresolvable and `resolve_images` runs again.
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

#[test]
fn intrinsic_size_changes_dirty_layout_caches_without_dirtying_unchanged_sizes() {
    let mut doc = Document::new();
    let root = doc.root_index();
    let img = doc.append_element(Some(root), "img", Style::default(), None::<&str>);
    doc.set_element_attributes(img, vec![("src".into(), "file:///x.png".into())]);
    doc.mark_in_document_flags();

    doc.layout_dirty = false;
    resolve_images(&mut doc, &FixedSizeResolver(10.0, 20.0)).expect("resolve Ok");
    assert!(doc.layout_dirty);
    assert_eq!(doc.nodes[img].image_intrinsic_size(), Some((10.0, 20.0)));

    doc.layout_dirty = false;
    resolve_images(&mut doc, &FixedSizeResolver(10.0, 20.0)).expect("resolve Ok");
    assert!(!doc.layout_dirty);

    resolve_images(&mut doc, &FixedSizeResolver(11.0, 20.0)).expect("resolve Ok");
    assert!(doc.layout_dirty);
    assert_eq!(doc.nodes[img].image_intrinsic_size(), Some((11.0, 20.0)));

    doc.layout_dirty = false;
    doc.set_element_attributes(img, vec![("src".into(), "relative.png".into())]);
    resolve_images(&mut doc, &FixedSizeResolver(11.0, 20.0)).expect("resolve Ok");
    assert!(doc.layout_dirty);
    assert_eq!(doc.nodes[img].image_intrinsic_size(), None);
}

#[test]
fn canvas_intrinsic_uses_width_height_with_defaults_and_ratio() {
    let mut doc = Document::new();
    let root = doc.root_index();
    let canvas = doc.append_element(Some(root), "canvas", Style::default(), None::<&str>);
    doc.mark_in_document_flags();
    resolve_canvas_intrinsic_sizes(&mut doc);
    assert_eq!(
        doc.nodes[canvas].image_intrinsic_size(),
        Some((300.0, 150.0))
    );
    doc.set_element_attributes(
        canvas,
        vec![
            ("width".into(), "50".into()),
            ("height".into(), "100".into()),
        ],
    );
    resolve_canvas_intrinsic_sizes(&mut doc);
    assert_eq!(
        doc.nodes[canvas].image_intrinsic_size(),
        Some((50.0, 100.0))
    );
    let intrinsic = doc.nodes[canvas]
        .image_intrinsic_box()
        .expect("intrinsic box");
    assert!((intrinsic.aspect_ratio.expect("ratio") - 0.5).abs() < 0.001);
}

#[test]
fn canvas_intrinsic_skips_non_canvas_and_inert_subtrees() {
    let mut doc = Document::new();
    let root = doc.root_index();
    let div = doc.append_element(Some(root), "div", Style::default(), None::<&str>);
    doc.set_element_attributes(div, vec![("width".into(), "50".into())]);
    doc.mark_in_document_flags();
    resolve_canvas_intrinsic_sizes(&mut doc);
    assert_eq!(doc.nodes[div].image_intrinsic_size(), None);
}

#[test]
fn canvas_intrinsic_handles_invalid_zero_overlong_and_foreign() {
    let mut doc = Document::new();
    let root = doc.root_index();
    let canvas = doc.append_element(Some(root), "canvas", Style::default(), None::<&str>);
    doc.set_element_attributes(
        canvas,
        vec![
            ("width".into(), "bogus".into()),
            ("height".into(), "000".into()),
        ],
    );
    doc.mark_in_document_flags();
    resolve_canvas_intrinsic_sizes(&mut doc);
    assert_eq!(doc.nodes[canvas].image_intrinsic_size(), Some((300.0, 0.0)));
    assert!(
        doc.nodes[canvas]
            .image_intrinsic_box()
            .expect("box")
            .aspect_ratio
            .is_none()
    );

    doc.set_element_attributes(
        canvas,
        vec![
            ("width".into(), "123456789012".into()),
            ("height".into(), "10".into()),
        ],
    );
    resolve_canvas_intrinsic_sizes(&mut doc);
    assert_eq!(
        doc.nodes[canvas].image_intrinsic_size(),
        Some((u32::MAX as f32, 10.0))
    );

    // Foreign-namespace canvas is not a canvas for intrinsic purposes.
    let foreign = doc.append_element(Some(root), "canvas", Style::default(), None::<&str>);
    doc.set_element_namespace(foreign, Some("http://www.w3.org/2000/svg".into()));
    doc.set_element_attributes(
        foreign,
        vec![
            ("width".into(), "50".into()),
            ("height".into(), "50".into()),
        ],
    );
    resolve_canvas_intrinsic_sizes(&mut doc);
    assert_eq!(doc.nodes[foreign].image_intrinsic_size(), None);
}

#[test]
fn resolved_urls_follow_relative_sources_base_changes_and_attribute_mutation() {
    let mut document = Document::new();
    let root = document.root_index();
    let image = document.append_element(Some(root), "img", Style::default(), None::<&str>);
    document.set_element_attributes(image, vec![("src".into(), "image.png".into())]);
    document.mark_in_document_flags();
    let first = Url::parse("https://first.test/sub/document.html").unwrap();
    resolve_images_with_base(&mut document, &FixedSizeResolver(4.0, 2.0), Some(&first)).unwrap();
    assert_eq!(
        document.resolved_image_url(image).unwrap().as_str(),
        "https://first.test/sub/image.png"
    );
    assert_eq!(document.nodes[image].attribute("src"), Some("image.png"));
    document.set_element_attributes(image, vec![("src".into(), "next.png".into())]);
    assert_eq!(document.resolved_image_url(image), None);
    let second = Url::parse("https://second.test/document.html").unwrap();
    resolve_images_with_base(&mut document, &FixedSizeResolver(4.0, 2.0), Some(&second)).unwrap();
    assert_eq!(
        document.resolved_image_url(image).unwrap().as_str(),
        "https://second.test/next.png"
    );
    document.set_element_attributes(image, Vec::new());
    assert_eq!(document.resolved_image_url(image), None);
    assert_eq!(document.resolved_image_url(usize::MAX), None);
}
