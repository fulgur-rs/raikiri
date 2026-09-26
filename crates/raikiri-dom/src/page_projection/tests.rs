use crate::{
    Document, PageContentInsets, PageMargins, PageSlice, layout_pages, layout_single_page,
    layout_single_page_with_resolver,
};
use parley::FontContext;
use raikiri_style::{CascadeResult, build_rule_tree, cascade};
use raikiri_traits::{
    NodeId, PageBox, ReplacedResolver, ResolvedIntrinsic, ResolverError, ResolverRequest,
};
use taffy::Style;

fn fixture() -> (
    Document,
    CascadeResult,
    PageBox,
    Vec<PageSlice>,
    usize,
    usize,
) {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = doc.append_element(Some(html), "body", Style::default(), Some("margin:0"));
    let div = doc.append_element(
        Some(body),
        "div",
        Style::default(),
        Some("width:10px;height:10px"),
    );
    let link = doc.append_element(Some(div), "a", Style::default(), None::<&str>);
    doc.set_element_attributes(link, vec![("href".into(), " /go ".into())]);
    let text = doc.append_text(link, "x");
    let fixed = doc.append_element(
        Some(body),
        "a",
        Style::default(),
        Some("position:fixed;top:0;width:10px;height:10px"),
    );
    doc.set_element_attributes(fixed, vec![("href".into(), "#fixed".into())]);
    doc.append_text(fixed, "f");
    let img = doc.append_element(Some(body), "img", Style::default(), None::<&str>);
    doc.set_element_attributes(
        img,
        vec![("src".into(), "https://example.test/image.png".into())],
    );
    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).expect("cascade");
    let mut page = PageBox::new();
    page.width = 100.0;
    page.height = 100.0;
    let slices = layout_pages(&mut doc, &cascade, page, FontContext::new()).expect("layout");
    (doc, cascade, page, slices, div, text)
}

fn project(doc: &mut Document, cascade: &CascadeResult, page: PageBox, slices: &[PageSlice]) {
    let geometries: Vec<_> = slices
        .iter()
        .map(|_| {
            (
                page,
                PageMargins {
                    top: 20.0,
                    right: 20.0,
                    bottom: 20.0,
                    left: 20.0,
                },
                PageContentInsets {
                    top: 5.0,
                    right: 5.0,
                    bottom: 5.0,
                    left: 5.0,
                },
            )
        })
        .collect();
    doc.project_pages(cascade, page, slices, &geometries);
}

#[test]
fn projection_view_adds_page_origin_once() {
    let (mut doc, cascade, page, slices, div, text) = fixture();
    project(&mut doc, &cascade, page, &slices[..1]);
    let fragment = doc
        .page_fragments(0)
        .find(|f| f.node() == NodeId(div as u64))
        .expect("div");
    let rect = fragment.rect();
    assert_eq!(
        (rect.x, rect.y, rect.width, rect.height),
        (25.0, 25.0, 10.0, 10.0)
    );
    assert_eq!(fragment.fragment_index(), 0);
    assert_eq!(fragment.fragmentainer(), 0);
    assert_eq!(fragment.is_first_fragment(), Some(true));
    assert_eq!(fragment.is_last_fragment(), Some(true));
    assert_eq!(fragment.repeat(), None);
    assert!(!fragment.continuation());
    let text = doc
        .page_fragments(0)
        .find(|f| f.node() == NodeId(text as u64))
        .expect("text");
    assert_eq!(text.line_range(), Some(0..1));
    let (_, target, quads) = doc
        .page_links(0)
        .find(|(_, target, _)| *target == "/go")
        .expect("link");
    assert_eq!(target, "/go");
    assert_eq!(quads, &[text.rect()]);
}

#[test]
fn projection_replaces_pages_and_links() {
    let (mut doc, cascade, page, slices, _, _) = fixture();
    let two = vec![
        slices[0].clone(),
        PageSlice {
            page_index: 1,
            content_origin_y: 100.0,
            page_name: None,
        },
    ];
    project(&mut doc, &cascade, page, &two);
    assert!(doc.page_fragments(1).next().is_some());
    assert!(doc.page_links(1).next().is_some());
    let count = doc.page_fragments(0).count();
    let links = doc.page_links(0).count();
    project(&mut doc, &cascade, page, &two[..1]);
    assert_eq!(doc.page_fragments(0).count(), count);
    assert_eq!(doc.page_links(0).count(), links);
    assert_eq!(doc.page_fragments(1).count(), 0);
    assert_eq!(doc.page_links(1).count(), 0);
    assert_eq!(doc.page_fragments(99).count(), 0);
    assert_eq!(doc.page_links(99).count(), 0);
}

#[test]
fn projection_is_cleared_by_mutation_and_relayout() {
    let (mut doc, cascade, page, slices, div, _) = fixture();
    project(&mut doc, &cascade, page, &slices[..1]);
    assert!(doc.page_fragments(0).next().is_some());
    doc.append_element(Some(div), "span", Style::default(), None::<&str>);
    assert_eq!(doc.page_fragments(0).count(), 0);
    assert_eq!(doc.page_links(0).count(), 0);
    let rules = build_rule_tree(&doc);
    let cascade = raikiri_style::cascade(&doc, &rules).expect("cascade");
    layout_single_page(&mut doc, &cascade, page, FontContext::new()).expect("relayout");
    project(&mut doc, &cascade, page, &slices[..1]);
    assert!(doc.page_links(0).next().is_some());
    layout_single_page(&mut doc, &cascade, page, FontContext::new()).expect("relayout");
    assert_eq!(doc.page_fragments(0).count(), 0);
    assert_eq!(doc.page_links(0).count(), 0);
}

#[test]
fn projection_is_cleared_before_a_resolver_error() {
    struct Fails;
    impl ReplacedResolver for Fails {
        fn resolve(&self, _: ResolverRequest<'_>) -> Result<ResolvedIntrinsic, ResolverError> {
            Err(ResolverError::Decode("failed".into()))
        }
    }
    let (mut doc, cascade, page, slices, _, _) = fixture();
    project(&mut doc, &cascade, page, &slices[..1]);
    assert!(doc.page_links(0).next().is_some());
    let error =
        layout_single_page_with_resolver(&mut doc, &cascade, page, FontContext::new(), &Fails)
            .expect_err("resolver failure");
    assert!(matches!(error, raikiri_traits::LayoutError::Resolver(_)));
    assert_eq!(doc.page_fragments(0).count(), 0);
    assert_eq!(doc.page_links(0).count(), 0);
}

#[test]
fn projection_clone_and_replacement_are_independent() {
    let (mut doc, cascade, page, slices, _, _) = fixture();
    project(&mut doc, &cascade, page, &slices[..1]);
    let count = doc.page_fragments(0).count();
    let links = doc.page_links(0).count();
    let mut cloned = doc.clone();
    assert_eq!(cloned.page_fragments(0).count(), count);
    assert_eq!(cloned.page_links(0).count(), links);
    project(&mut cloned, &cascade, page, &[]);
    assert_eq!(cloned.page_fragments(0).count(), 0);
    assert_eq!(cloned.page_links(0).count(), 0);
    assert_eq!(doc.page_fragments(0).count(), count);
    assert_eq!(doc.page_links(0).count(), links);
}

fn assert_paged_resolver_error_clears_projection(with_geometry: bool) {
    struct Fails;
    impl ReplacedResolver for Fails {
        fn resolve(&self, _: ResolverRequest<'_>) -> Result<ResolvedIntrinsic, ResolverError> {
            Err(ResolverError::Decode("failed".into()))
        }
    }
    let (mut doc, cascade, page, slices, _, _) = fixture();
    project(&mut doc, &cascade, page, &slices[..1]);
    assert!(doc.page_fragments(0).next().is_some());
    assert!(doc.page_links(0).next().is_some());
    let result = if with_geometry {
        crate::layout_pages_with_page_geometry_and_resolver_and_base_url(
            &mut doc,
            &cascade,
            page,
            FontContext::new(),
            &[100.0],
            &[100.0],
            &Fails,
            None,
        )
    } else {
        crate::layout_pages_with_resolver_and_base_url(
            &mut doc,
            &cascade,
            page,
            FontContext::new(),
            &Fails,
            None,
        )
    };
    assert!(matches!(
        result,
        Err(raikiri_traits::LayoutError::Resolver(_))
    ));
    assert_eq!(doc.page_fragments(0).count(), 0);
    assert_eq!(doc.page_links(0).count(), 0);
}

#[test]
fn projection_is_cleared_before_paged_resolver_error() {
    assert_paged_resolver_error_clears_projection(false);
}

#[test]
fn projection_is_cleared_before_scheduled_resolver_error() {
    assert_paged_resolver_error_clears_projection(true);
}

fn assert_metadata_change_clears_projection(change: u8) {
    let (mut doc, cascade, page, slices, div, _) = fixture();
    project(&mut doc, &cascade, page, &slices[..1]);
    let owner = doc
        .page_links(0)
        .find(|(_, target, _)| *target == "/go")
        .expect("link")
        .0
        .0 as usize;
    let layout_dirty = doc.layout_dirty;
    assert!(doc.page_fragments(0).next().is_some());
    match change {
        0 => doc.set_element_attributes(owner, vec![("href".into(), "/new".into())]),
        1 => doc
            .set_element_attribute(owner, "href", "/new")
            .expect("set href"),
        2 => {
            assert!(
                doc.remove_element_attribute(owner, "href")
                    .expect("remove href")
                    .is_some()
            );
        }
        3 => doc.set_element_inline_style(div, Some("width:40px".into())),
        4 => doc.set_element_namespace(div, Some("http://www.w3.org/2000/svg".into())),
        5 => doc
            .set_element_namespaced_attribute(owner, "urn:example", None, "href", "/new")
            .expect("qualified attribute"),
        6 => doc
            .set_element_attribute(div, "style", "width:40px")
            .expect("set style"),
        7 => {
            assert!(
                doc.remove_element_attribute(div, "style")
                    .expect("remove style")
                    .is_some()
            );
        }
        _ => unreachable!(),
    }
    assert_eq!(doc.layout_dirty, layout_dirty);
    assert_eq!(doc.page_fragments(0).count(), 0);
    assert_eq!(doc.page_links(0).count(), 0);
}

#[test]
fn projection_is_cleared_by_attribute_list_change() {
    assert_metadata_change_clears_projection(0);
}
#[test]
fn projection_is_cleared_by_href_change() {
    assert_metadata_change_clears_projection(1);
}
#[test]
fn projection_is_cleared_by_href_removal() {
    assert_metadata_change_clears_projection(2);
}
#[test]
fn projection_is_cleared_by_inline_style_change() {
    assert_metadata_change_clears_projection(3);
}
#[test]
fn projection_is_cleared_by_namespace_change() {
    assert_metadata_change_clears_projection(4);
}
#[test]
fn projection_is_cleared_by_namespaced_attribute_change() {
    assert_metadata_change_clears_projection(5);
}
#[test]
fn projection_is_cleared_by_style_attribute_change() {
    assert_metadata_change_clears_projection(6);
}
#[test]
fn projection_is_cleared_by_style_attribute_removal() {
    assert_metadata_change_clears_projection(7);
}

#[test]
fn projection_is_cleared_by_text_relayout() {
    let (mut doc, cascade, page, slices, _, _) = fixture();
    project(&mut doc, &cascade, page, &slices[..1]);
    assert!(doc.page_fragments(0).next().is_some());
    assert!(doc.page_links(0).next().is_some());
    crate::relayout_text_for_width(&mut doc, &cascade, 5.0, page.width, FontContext::new());
    assert_eq!(doc.page_fragments(0).count(), 0);
    assert_eq!(doc.page_links(0).count(), 0);
}
