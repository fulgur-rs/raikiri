// Geometry expectations were measured with Chromium 152.0.7977.82, the
// repository Ahem font at 20px/30px, square SVG images and zero page margins.
// Page-fit cases deliberately keep the first line's original 30px fit extent.
use super::*;
use raikiri_traits::{DecodedImage, ImagePixelSource, PageBox};
use std::sync::Arc;

struct Pixels(u32);
impl ImagePixelSource for Pixels {
    fn get_decoded(&self, url: &url::Url) -> Option<Arc<DecodedImage>> {
        let size = if url.path() == "/child.png" {
            5
        } else {
            self.0
        };
        Some(Arc::new(DecodedImage::new(
            size,
            size,
            [255, 0, 0, 255].repeat((size * size) as usize),
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
    fixture_with_page(prefix, size, 100.0, configure)
}

fn fixture_with_page(
    prefix: u32,
    size: u32,
    height: f32,
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
    page.height = height;
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
fn a_tall_image_enlarges_the_line_and_follows_it_to_the_next_page() {
    let (doc, _, owner) = fixture_with(85, 80, |_, _| {});
    let id = NodeId::new(owner as u64);
    assert!(doc.page_marker_image(0, id).is_none());
    let (_, rect) = doc.page_marker_image(1, id).unwrap();
    assert_eq!(rect.y + rect.height, 80.0);
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

#[test]
fn oversized_outside_image_enlarges_the_first_line_metrics() {
    // Chromium 152 with Ahem 20px/30px: the 30px replaced marker raises
    // the first line baseline from 86px to 95px, after the 65px block.
    let (doc, cascade, owner) = fixture_with(65, 30, |_, _| {});
    let (_, rect) = doc.page_marker_image(0, NodeId::new(owner as u64)).unwrap();
    assert_eq!((rect.y, rect.height), (65.0, 30.0));
    let text = doc.page_text_runs(&cascade, 0);
    let run = text.iter().find(|run| run.text.contains('z')).unwrap();
    assert_eq!(run.origin.1, 95.0);
    let line = &doc.nodes[owner]
        .ifc
        .as_ref()
        .unwrap()
        .lines
        .as_ref()
        .unwrap()
        .lines[0];
    assert_eq!(
        line.block_offset() + line.baseline(shodo::geometry::BaselineKind::Alphabetic),
        95.0
    );
    assert_eq!(line.block_size(), 39.0);
}

#[test]
fn outside_image_leading_moves_a_child_box_without_changing_its_line_metrics() {
    for (style, prefix, expected_top, expected_height, expected_baseline) in [
        ("display:block", 0, 9.0, 30.0, 30.0),
        ("display:block;padding-top:20px", 0, 0.0, 50.0, 41.0),
        ("display:block", 65, 74.0, 30.0, 95.0),
        ("display:flex", 0, 9.0, 30.0, 30.0),
        ("display:grid", 0, 9.0, 30.0, 30.0),
        ("display:flow-root", 0, 9.0, 30.0, 30.0),
    ] {
        let (doc, cascade, owner) = fixture_with_page(prefix, 30, 300.0, |doc, owner| {
            let text = *doc.nodes[owner].children.last().unwrap();
            doc.detach_from_parent(text);
            let child = doc.append_element(Some(owner), "div", Some(style));
            doc.append_text(child, "z");
        });
        let child = *doc.nodes[owner].children.last().unwrap();
        assert_eq!(
            doc.nodes[child].layout.location.y, expected_top,
            "{style}, prefix {prefix}"
        );
        assert_eq!(
            doc.nodes[child].layout.size.height, expected_height,
            "{style}"
        );
        let runs = doc.page_text_runs(&cascade, 0);
        let run = runs.iter().find(|run| run.text.contains('z')).unwrap();
        assert_eq!(run.origin.1, expected_baseline, "{style}");
    }
}

#[test]
fn a_childs_internal_prefix_counts_in_its_exported_marker_baseline() {
    let (doc, cascade, owner) = fixture_with_page(0, 30, 300.0, |doc, owner| {
        let text = *doc.nodes[owner].children.last().unwrap();
        doc.detach_from_parent(text);
        let child = doc.append_element(Some(owner), "div", Some("display:block"));
        doc.append_element(Some(child), "div", Some("display:block;height:65px"));
        doc.append_text(child, "z");
    });
    let child = *doc.nodes[owner].children.last().unwrap();
    assert_eq!(doc.nodes[child].layout.location.y, 0.0);
    assert_eq!(doc.nodes[child].layout.size.height, 95.0);
    let runs = doc.page_text_runs(&cascade, 0);
    assert_eq!(
        runs.iter()
            .find(|run| run.text.contains('z'))
            .unwrap()
            .origin
            .1,
        86.0
    );
}

#[test]
fn outside_marker_leading_preserves_negative_leading_and_only_moves_the_first_line() {
    for (line_height, expected_height) in [(0, 24.0), (10, 29.0), (30, 39.0), (100, 100.0)] {
        let (doc, cascade, owner) = fixture_with_page(0, 30, 300.0, |doc, owner| {
            doc.set_element_attribute(owner, "style", format!("display:list-item;list-style:outside url(https://images.test/marker.png);line-height:{line_height}px")).unwrap();
        });
        assert_eq!(
            doc.nodes[owner].layout.size.height, expected_height,
            "line-height {line_height}"
        );
        let runs = doc.page_text_runs(&cascade, 0);
        let baseline = runs
            .iter()
            .find(|run| run.text.contains('z'))
            .unwrap()
            .origin
            .1;
        assert_eq!(baseline, if line_height == 100 { 56.0 } else { 30.0 });
    }
    let (doc, cascade, owner) = fixture_with_page(0, 30, 300.0, |doc, owner| {
        doc.append_element(Some(owner), "br", None::<&str>);
        doc.append_text(owner, "q");
    });
    let runs = doc.page_text_runs(&cascade, 0);
    assert_eq!(
        runs.iter()
            .find(|run| run.text.contains('z'))
            .unwrap()
            .origin
            .1,
        30.0
    );
    assert_eq!(
        runs.iter()
            .find(|run| run.text.contains('q'))
            .unwrap()
            .origin
            .1,
        60.0
    );
    assert_eq!(doc.nodes[owner].layout.size.height, 69.0);
}

#[test]
fn marker_leading_changes_advance_but_page_fit_keeps_the_original_line_height() {
    for (prefix, size, page, baseline) in [
        (65, 30, 0, 95.0),
        (70, 30, 0, 100.0),
        (71, 30, 1, 30.0),
        (65, 50, 0, 115.0),
        (30, 80, 0, 110.0),
        (85, 80, 1, 80.0),
    ] {
        let (doc, _, owner) = fixture_with(prefix, size, |_, _| {});
        let id = NodeId::new(owner as u64);
        let (_, rect) = doc.page_marker_image(page, id).unwrap();
        assert_eq!(
            rect.y + rect.height,
            baseline,
            "prefix {prefix}, size {size}"
        );
        assert!(
            doc.page_marker_image(1 - page, id).is_none(),
            "prefix {prefix}, size {size}"
        );
    }
}

#[test]
fn a_float_on_the_markers_first_line_moves_with_its_leading() {
    let (doc, cascade, owner) = fixture_with_page(0, 30, 300.0, |doc, owner| {
        let text = *doc.nodes[owner].children.last().unwrap();
        doc.detach_from_parent(text);
        doc.append_element(Some(owner), "div", Some("float:left;width:20px;height:5px"));
        doc.append_text(owner, "z");
    });
    let floated = doc.nodes[owner].children[1];
    assert_eq!(doc.nodes[floated].layout.location.y, 9.0);
    let runs = doc.page_text_runs(&cascade, 0);
    let text = runs.iter().find(|run| run.text.contains('z')).unwrap();
    assert_eq!(text.origin, (60.0, 30.0));
}

#[test]
fn empty_marker_lines_expand_but_a_line_free_block_does_not() {
    for empty in [false, true] {
        let (doc, _, owner) = fixture_with_page(65, 30, 300.0, |doc, owner| {
            let text = *doc.nodes[owner].children.last().unwrap();
            doc.detach_from_parent(text);
            if empty {
                let block = doc.nodes[owner].children[0];
                doc.detach_from_parent(block);
            }
        });
        assert_eq!(
            doc.nodes[owner].layout.size.height,
            if empty { 39.0 } else { 65.0 }
        );
        let (_, rect) = doc.page_marker_image(0, NodeId::new(owner as u64)).unwrap();
        assert_eq!(rect.y, 0.0);
    }
}

#[test]
fn outside_image_uses_the_resolved_font_and_line_height() {
    for (font, line_height, expected_height) in [(10, 10, 32.0), (20, 30, 39.0), (25, 35, 40.0)] {
        let (doc, cascade, owner) = fixture_with_page(0, 30, 300.0, |doc, owner| {
            doc.set_element_attribute(owner,"style",format!("display:list-item;list-style:outside url(https://images.test/marker.png);font:{font}px/{line_height}px Ahem")).unwrap();
        });
        assert_eq!(doc.nodes[owner].layout.size.height, expected_height);
        let runs = doc.page_text_runs(&cascade, 0);
        assert_eq!(
            runs.iter()
                .find(|run| run.text.contains('z'))
                .unwrap()
                .origin
                .1,
            30.0
        );
    }
}

#[test]
fn a_generated_block_has_the_same_marker_leading_as_a_real_child_box() {
    for pseudo in ["before", "after"] {
        for padding in [0, 20] {
            let (doc, cascade, owner) = fixture_with_page(0, 30, 300.0, |doc, owner| {
                let text = *doc.nodes[owner].children.last().unwrap();
                doc.detach_from_parent(text);
                let sheet = doc.append_element(Some(0), "style", None::<&str>);
                doc.append_text(sheet,format!("li::{pseudo}{{content:'z';display:block;padding-top:{padding}px;background:blue}}"));
            });
            let baseline = if padding == 0 { 30.0 } else { 41.0 };
            let (_, marker) = doc.page_marker_image(0, NodeId::new(owner as u64)).unwrap();
            assert_eq!(
                marker.y + marker.height,
                baseline,
                "{pseudo}, padding {padding}"
            );
            let runs = doc.page_text_runs(&cascade, 0);
            assert_eq!(
                runs.iter()
                    .find(|run| run.text.contains('z'))
                    .unwrap()
                    .origin
                    .1,
                baseline
            );
            let block = &doc.nodes[owner]
                .ifc
                .as_ref()
                .unwrap()
                .lines
                .as_ref()
                .unwrap()
                .placed_blocks[0];
            assert_eq!(
                block.border_box.y,
                if padding == 0 { 9.0 } else { 0.0 },
                "{pseudo}, padding {padding}"
            );
            assert_eq!(
                block.border_box.height,
                30.0 + padding as f32,
                "{pseudo}, padding {padding}"
            );
        }
    }
}

#[test]
fn a_childs_marker_leading_does_not_change_page_fit_of_its_first_line() {
    let (doc, cascade, owner) = fixture_with(65, 30, |doc, owner| {
        let text = *doc.nodes[owner].children.last().unwrap();
        doc.detach_from_parent(text);
        let child = doc.append_element(Some(owner), "div", Some("display:block"));
        doc.append_text(child, "z");
    });
    let (_, marker) = doc.page_marker_image(0, NodeId::new(owner as u64)).unwrap();
    assert_eq!(marker.y, 65.0);
    let runs = doc.page_text_runs(&cascade, 0);
    assert_eq!(
        runs.iter()
            .find(|run| run.text.contains('z'))
            .unwrap()
            .origin
            .1,
        95.0
    );
}

#[test]
fn a_marker_gap_rechecks_a_child_box_against_later_float_bands() {
    let (doc, cascade, owner) = fixture_with_page(0, 30, 300.0, |doc, owner| {
        let list = doc.nodes[owner].parent.unwrap();
        let body = doc.nodes[list].parent.unwrap();
        let float = doc.append_element(
            Some(body),
            "div",
            Some("display:block;float:left;width:60px;height:30px;margin-top:35px"),
        );
        doc.detach_from_parent(float);
        doc.insert_child_before(body, list, float);
        let text = *doc.nodes[owner].children.last().unwrap();
        doc.detach_from_parent(text);
        let child = doc.append_element(Some(owner), "div", Some("display:flow-root"));
        doc.append_text(child, "z");
    });
    let child = *doc.nodes[owner].children.last().unwrap();
    assert_eq!(doc.nodes[child].layout.location.x, 20.0);
    assert_eq!(doc.nodes[child].layout.location.y, 9.0);
    let runs = doc.page_text_runs(&cascade, 0);
    let run = runs.iter().find(|run| run.text.contains('z')).unwrap();
    assert_eq!(run.origin, (60.0, 30.0));
}

#[test]
fn a_child_marker_gap_only_changes_the_first_line_fit_and_survives_cached_layout() {
    let (mut doc, cascade, owner) = fixture_with(65, 30, |doc, owner| {
        let text = *doc.nodes[owner].children.last().unwrap();
        doc.detach_from_parent(text);
        let child =
            doc.append_element(Some(owner), "div", Some("display:block;orphans:1;widows:1"));
        doc.append_text(child, "z");
        doc.append_element(Some(child), "br", None::<&str>);
        doc.append_text(child, "q");
    });
    let child = *doc.nodes[owner].children.last().unwrap();
    assert_eq!(
        crate::layout::ifc::outside_marker::fit_leading(&doc, child, 0),
        9.0
    );
    assert_eq!(
        crate::layout::ifc::outside_marker::fit_leading(&doc, child, 1),
        0.0
    );
    let mut page = PageBox::new();
    page.width = 100.0;
    page.height = 100.0;
    let slices = crate::layout_pages(&mut doc, &cascade, page).unwrap();
    doc.project_pages(&cascade, page, &slices, &[]).unwrap();
    assert_eq!(
        doc.page_marker_image(0, NodeId::new(owner as u64))
            .unwrap()
            .1
            .y,
        65.0
    );
    assert!(
        doc.page_text_runs(&cascade, 0)
            .iter()
            .any(|run| run.text.contains('z'))
    );
    assert!(
        !doc.page_text_runs(&cascade, 0)
            .iter()
            .any(|run| run.text.contains('q'))
    );
    assert!(
        doc.page_text_runs(&cascade, 1)
            .iter()
            .any(|run| run.text.contains('q'))
    );
}

#[test]
fn moving_a_child_out_of_a_hidden_marker_owner_drops_its_old_gap() {
    let (mut doc, _, owner) = fixture_with_page(0, 30, 300.0, |doc, owner| {
        let text = *doc.nodes[owner].children.last().unwrap();
        doc.detach_from_parent(text);
        let child = doc.append_element(Some(owner), "div", Some("display:block"));
        doc.append_text(child, "z");
    });
    let child = *doc.nodes[owner].children.last().unwrap();
    let body = doc.nodes[doc.nodes[owner].parent.unwrap()].parent.unwrap();
    assert_eq!(
        crate::layout::ifc::outside_marker::fit_leading(&doc, child, 0),
        9.0
    );
    doc.detach_from_parent(child);
    doc.attach_child(body, child);
    doc.set_element_attribute(owner, "style", "display:none")
        .unwrap();
    let cascade = raikiri_style::cascade(&doc, &raikiri_style::build_rule_tree(&doc)).unwrap();
    doc.prepare_list_marker_images(&cascade, &Pixels(30), None);
    let mut page = PageBox::new();
    page.width = 100.0;
    page.height = 300.0;
    let slices = crate::layout_pages(&mut doc, &cascade, page).unwrap();
    doc.project_pages(&cascade, page, &slices, &[]).unwrap();
    assert_eq!(
        crate::layout::ifc::outside_marker::fit_leading(&doc, child, 0),
        0.0
    );
    let runs = doc.page_text_runs(&cascade, 0);
    assert_eq!(
        runs.iter()
            .find(|run| run.text.contains('z'))
            .unwrap()
            .origin
            .1,
        21.0
    );
}

#[test]
fn a_multicol_item_keeps_marker_leading_outside_its_column_lines() {
    let (doc, cascade, owner) = fixture_with_page(0, 30, 300.0, |doc, owner| {
        let prefix = doc.nodes[owner].children[0];
        doc.detach_from_parent(prefix);
        doc.set_element_attribute(owner, "style", "display:list-item;list-style:outside url(https://images.test/marker.png);column-count:2;column-gap:0;column-fill:auto;height:30px;orphans:1;widows:1").unwrap();
        doc.append_element(Some(owner), "br", None::<&str>);
        doc.append_text(owner, "q");
    });
    let runs = doc.page_text_runs(&cascade, 0);
    assert_eq!(
        runs.iter()
            .find(|run| run.text.contains('z'))
            .unwrap()
            .origin
            .1,
        21.0
    );
    assert_eq!(
        runs.iter()
            .find(|run| run.text.contains('q'))
            .unwrap()
            .origin
            .1,
        21.0
    );
    assert_eq!(
        doc.page_marker_image(0, NodeId::new(owner as u64))
            .unwrap()
            .1
            .y,
        0.0
    );
}

#[test]
fn a_generated_only_child_uses_the_same_first_line_fit() {
    let (doc, cascade, owner) = fixture_with(65, 30, |doc, owner| {
        let text = *doc.nodes[owner].children.last().unwrap();
        doc.detach_from_parent(text);
        let child = doc.append_element(Some(owner), "div", Some("display:block"));
        doc.set_element_attribute(child, "id", "marker-child")
            .unwrap();
        let sheet = doc.append_element(Some(0), "style", None::<&str>);
        doc.append_text(sheet, "#marker-child::before{content:'z'}");
    });
    assert_eq!(
        doc.page_marker_image(0, NodeId::new(owner as u64))
            .unwrap()
            .1
            .y,
        65.0
    );
    assert_eq!(
        doc.page_text_runs(&cascade, 0)
            .iter()
            .find(|run| run.text.contains('z'))
            .unwrap()
            .origin
            .1,
        95.0
    );
}

#[test]
fn a_childs_generated_image_keeps_the_first_line_page() {
    let (doc, _, owner) = fixture_with(65, 50, |doc, owner| {
        let text = *doc.nodes[owner].children.last().unwrap();
        doc.detach_from_parent(text);
        let child = doc.append_element(Some(owner), "div", Some("display:block"));
        doc.set_element_attribute(child, "id", "marker-child")
            .unwrap();
        doc.append_text(child, "z");
        let sheet = doc.append_element(Some(0), "style", None::<&str>);
        doc.append_text(sheet, "#marker-child::before{content:url(https://images.test/child.png);display:inline-block;width:5px;height:5px;line-height:0;vertical-align:top}");
    });
    let child = *doc.nodes[owner].children.last().unwrap();
    let image = crate::GeneratedImageId::new(child, false, 0);
    let (_, rect) = doc.page_generated_image(0, image).unwrap();
    assert_eq!((rect.y, rect.height), (95.0, 5.0));
}

#[test]
fn multicol_marker_alignment_respects_padding_and_preceding_content() {
    for (padding, prefix, size, count, expected) in [
        (20, 0, 30, 2, 20.0),
        (0, 65, 30, 2, 56.0),
        (0, 0, 20, 2, 1.0),
        (0, 0, 30, 1, 0.0),
    ] {
        let (doc, _, owner) = fixture_with_page(prefix, size, 300.0, |doc, owner| {
            doc.set_element_attribute(owner, "style", format!("display:list-item;list-style:outside url(https://images.test/marker.png);column-count:{count};column-gap:0;column-fill:auto;height:100px;padding-top:{padding}px")).unwrap();
        });
        assert_eq!(
            doc.page_marker_image(0, NodeId::new(owner as u64))
                .unwrap()
                .1
                .y,
            expected,
            "padding {padding}, prefix {prefix}, size {size}, count {count}"
        );
    }
}
