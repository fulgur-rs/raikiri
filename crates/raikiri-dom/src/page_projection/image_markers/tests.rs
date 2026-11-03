use super::*;
use raikiri_traits::{DecodedImage, ImagePixelSource, PageBox};
use std::sync::Arc;

struct Pixels(u32);
impl ImagePixelSource for Pixels {
    fn get_decoded(&self, _: &url::Url) -> Option<Arc<DecodedImage>> {
        Some(Arc::new(DecodedImage::new(
            self.0,
            self.0,
            [255, 0, 0, 255].repeat((self.0 * self.0) as usize),
        )))
    }
}

fn fixture(prefix: u32) -> (Document, CascadeResult, usize) {
    fixture_with(prefix, 20, |_, _| {})
}

fn fixture_with(
    prefix: u32,
    size: u32,
    configure: impl FnOnce(&mut Document, usize),
) -> (Document, CascadeResult, usize) {
    let mut doc = Document::new();
    crate::layout::test_support::with_ahem(&mut doc);
    let body = doc.append_element(
        Some(0),
        "body",
        Some("display:block;margin:0;font:20px/30px Ahem"),
    );
    let list = doc.append_element(
        Some(body),
        "ul",
        Some("display:block;margin:0;padding:0 0 0 40px"),
    );
    let owner = doc.append_element(
        Some(list),
        "li",
        Some("display:list-item;list-style:outside url(https://images.test/marker.png)"),
    );
    doc.append_element(
        Some(owner),
        "div",
        Some(format!("display:block;height:{prefix}px")),
    );
    doc.append_text(owner, "z");
    configure(&mut doc, owner);
    let cascade = raikiri_style::cascade(&doc, &raikiri_style::build_rule_tree(&doc)).unwrap();
    doc.prepare_list_marker_images(&cascade, &Pixels(size), None);
    let mut page = PageBox::new();
    page.width = 100.0;
    page.height = 100.0;
    let slices = crate::layout_pages(&mut doc, &cascade, page).unwrap();
    doc.project_pages(&cascade, page, &slices, &[]).unwrap();
    (doc, cascade, owner)
}

#[test]
fn outside_image_follows_the_first_line_on_its_page() {
    // Chromium 152, embedded Ahem 20px/30px and a 20px square image:
    // the image bottom meets the first line baseline, 21px below its top.
    let (doc, _, owner) = fixture(65);
    let (_, rect) = doc.page_marker_image(0, NodeId::new(owner as u64)).unwrap();
    assert_eq!((rect.y, rect.width, rect.height), (66.0, 20.0, 20.0));
}

#[test]
fn outside_image_moves_with_the_line_to_the_next_page() {
    let (doc, cascade, owner) = fixture(85);
    let id = NodeId::new(owner as u64);
    assert!(doc.page_marker_image(0, id).is_none());
    let (_, rect) = doc.page_marker_image(1, id).unwrap();
    assert_eq!((rect.y, rect.width, rect.height), (1.0, 20.0, 20.0));
    assert!(
        doc.page_text_runs(&cascade, 0)
            .iter()
            .all(|run| !run.text.contains('z'))
    );
    assert!(
        doc.page_text_runs(&cascade, 1)
            .iter()
            .any(|run| run.text.contains('z'))
    );
}

#[test]
fn image_height_changes_its_top_but_not_its_baseline() {
    for size in [5, 10, 20] {
        let (doc, _, owner) = fixture_with(65, size, |_, _| {});
        let (_, rect) = doc.page_marker_image(0, NodeId::new(owner as u64)).unwrap();
        assert_eq!(rect.y + rect.height, 86.0);
    }
}

#[test]
fn prepared_top_is_shared_with_the_builtin_painter() {
    let (doc, _, owner) = fixture(85);
    assert_eq!(
        doc.list_marker_image_top(owner, (0.0, false), 0.0),
        Some(None)
    );
    assert_eq!(
        doc.list_marker_image_top(owner, (100.0, false), -100.0),
        Some(Some(1.0))
    );
    assert_eq!(
        doc.list_marker_image_top(owner, (0.0, true), 0.0),
        Some(Some(101.0))
    );
    assert_eq!(doc.list_marker_image_top(0, (0.0, false), 0.0), None);
}

#[test]
fn a_fixed_image_repeats_at_its_aligned_top() {
    let (doc, _, owner) = fixture_with(85, 20, |doc, owner| {
        doc.set_element_attribute(owner, "style", "display:list-item;list-style:outside url(https://images.test/marker.png);position:fixed;top:0;left:40px;width:60px").unwrap();
        let body = doc.parent_of(doc.parent_of(owner).unwrap()).unwrap();
        doc.append_element(Some(body), "div", Some("display:block;height:200px"));
    });
    let id = NodeId::new(owner as u64);
    for page in [0, 1] {
        let (_, rect) = doc.page_marker_image(page, id).unwrap();
        assert_eq!(rect.y, 86.0);
    }
}

#[test]
fn child_relative_position_does_not_move_the_marker() {
    let (doc, _, owner) = fixture_with(0, 20, |doc, owner| {
        let text = doc.nodes[owner].children.last().copied().unwrap();
        doc.detach_from_parent(text);
        let child = doc.append_element(
            Some(owner),
            "div",
            Some("display:block;position:relative;top:20px"),
        );
        doc.append_text(child, "z");
    });
    let (_, rect) = doc.page_marker_image(0, NodeId::new(owner as u64)).unwrap();
    assert_eq!(rect.y, 1.0);
}

#[test]
fn a_line_free_item_keeps_its_original_image_top() {
    let (doc, _, owner) = fixture_with(65, 20, |doc, owner| {
        let text = *doc.nodes[owner].children.last().unwrap();
        doc.detach_from_parent(text);
    });
    let (_, rect) = doc.page_marker_image(0, NodeId::new(owner as u64)).unwrap();
    assert_eq!(rect.y, 0.0);
}

#[test]
fn inside_images_keep_their_inline_placement() {
    let (doc, _, owner) = fixture_with(0, 20, |doc, owner| {
        doc.set_element_attribute(
            owner,
            "style",
            "display:list-item;list-style:inside url(https://images.test/marker.png)",
        )
        .unwrap();
    });
    assert!(
        doc.page_marker_image(0, NodeId::new(owner as u64))
            .is_some()
    );
    assert_eq!(doc.list_marker_image_top(owner, (0.0, false), 0.0), None);
}

#[test]
fn a_tall_image_follows_the_engine_line_baseline_and_page() {
    // This checks placement against the engine line metrics, not browser
    // parity: oversized outside images do not yet enlarge those metrics.
    let (doc, _, owner) = fixture_with(85, 80, |_, _| {});
    let id = NodeId::new(owner as u64);
    assert!(doc.page_marker_image(0, id).is_none());
    let (_, rect) = doc.page_marker_image(1, id).unwrap();
    assert_eq!(rect.y + rect.height, 21.0);
}

#[test]
fn vertical_images_keep_the_existing_physical_top() {
    for mode in ["vertical-rl", "vertical-lr", "sideways-rl", "sideways-lr"] {
        let (doc, _, owner) = fixture_with(0, 20, |doc, owner| {
            doc.set_element_attribute(owner, "style", format!("display:list-item;list-style:outside url(https://images.test/marker.png);writing-mode:{mode};width:60px;height:60px")).unwrap();
        });
        let (_, rect) = doc.page_marker_image(0, NodeId::new(owner as u64)).unwrap();
        assert_eq!(rect.y, 0.0, "{mode}");
    }
}

#[test]
fn prepared_images_clear_with_the_projection() {
    let (mut doc, _, owner) = fixture(65);
    assert!(
        doc.list_marker_image_top(owner, (0.0, false), 0.0)
            .is_some()
    );
    doc.page_projection.clear();
    assert_eq!(doc.list_marker_image_top(owner, (0.0, false), 0.0), None);
    assert!(
        doc.page_marker_image(0, NodeId::new(owner as u64))
            .is_none()
    );
}

#[test]
fn standalone_image_work_does_not_multiply_by_page_count() {
    let (doc, cascade, _) = fixture(65);
    let spent = |count: u32| {
        let pages: Vec<_> = (0..count)
            .map(|i| {
                let mut page = doc.page_projection.pages[0].clone();
                page.page_index = i;
                page.content_origin_y = i as f32 * 100.0;
                page.flow_range = Some((i as f32 * 100.0, (i + 1) as f32 * 100.0));
                page
            })
            .collect();
        let control = crate::PageLayoutControl::default();
        let budget = 10_000;
        let mut work = crate::layout::ProjectionWork::new(&control, budget);
        let prepared = prepare(
            &doc,
            &cascade,
            &doc.page_projection.text_roots,
            &pages,
            &Default::default(),
            &mut work,
        )
        .unwrap();
        assert_eq!(prepared.pages.values().map(BTreeMap::len).sum::<usize>(), 1);
        budget - work.remaining()
    };
    assert_eq!(spent(50), spent(200));
}

#[test]
fn an_orthogonal_first_child_keeps_the_existing_fallback() {
    // This preserves the existing engine placement; Chromium's image top
    // is 1px here. Its orthogonal baseline is outside this change's scope.
    for trailing_text in [false, true] {
        let (doc, _, owner) = fixture_with(0, 20, |doc, owner| {
            let text = *doc.nodes[owner].children.last().unwrap();
            doc.detach_from_parent(text);
            let child = doc.append_element(
                Some(owner),
                "div",
                Some("display:block;writing-mode:vertical-rl;width:30px;height:40px"),
            );
            doc.append_text(child, "z");
            if trailing_text {
                doc.append_text(owner, "z");
            }
        });
        let (_, rect) = doc.page_marker_image(0, NodeId::new(owner as u64)).unwrap();
        assert_eq!(rect.y, 0.0);
    }
}

#[test]
fn multiple_roots_keep_the_first_fragment_page() {
    let (doc, cascade, owner) = fixture(85);
    let marker = doc
        .page_projection
        .text_roots
        .iter()
        .find(|root| generated_origin(root.node) == Some((owner, PseudoElem::Marker)))
        .copied()
        .unwrap();
    let roots = [marker, marker];
    let control = crate::PageLayoutControl::default();
    let mut work = crate::layout::ProjectionWork::new(&control, 10_000);
    let prepared = prepare(
        &doc,
        &cascade,
        &roots,
        &doc.page_projection.pages,
        &Default::default(),
        &mut work,
    )
    .unwrap();
    assert!(prepared.pages[&0].contains_key(&NodeId::new(owner as u64)));
    assert!(prepared.pages[&1].is_empty());
    let mut work = crate::layout::ProjectionWork::new(&control, 10_000);
    let absent = prepare(&doc, &cascade, &roots, &[], &Default::default(), &mut work).unwrap();
    assert!(absent.pages.is_empty());
}
