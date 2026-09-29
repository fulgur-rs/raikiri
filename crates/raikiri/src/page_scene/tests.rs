use super::*;
use crate::build_cascaded;
use parley::FontContext;
use raikiri_html::{ParseOptions, parse};

/// Parse, cascade, and lay out a hello-world HTML document, then return
/// the post-layout Document and CascadeResult. Shared smoke-test setup
/// for build_page_scene.
fn hello_world_post_layout() -> (Document, CascadeResult) {
    let opts = ParseOptions {
        extra_stylesheets: &[],
        network: None,
        base_url: None,
    };
    let uncascaded = parse(&b"<p>Hi</p>"[..], &opts).expect("parse Ok");
    let cascade = build_cascaded(&uncascaded);
    let mut dom = uncascaded.dom;
    raikiri_dom::layout_single_page(&mut dom, &cascade, PageBox::A4, FontContext::new())
        .expect("layout Ok");
    (dom, cascade)
}

/// Check that build_page_scene populates metadata and fragments from a
/// post-layout hello-world Document. The recommended integration assertions
/// require nonempty node_ids and populated body_id/root_id, preventing
/// this path from going untested.
#[test]
fn build_page_scene_populates_metadata_from_hello_world() {
    let (dom, cascade) = hello_world_post_layout();
    let scene = build_page_scene(&dom, &cascade, PageBox::A4);

    assert!(
        scene.root_id.is_some(),
        "root_id must resolve to <html> for well-formed document"
    );
    assert!(
        scene.body_id.is_some(),
        "body_id must resolve to <body> for well-formed document"
    );
    assert!(
        !scene.node_ids.is_empty(),
        "node_ids must include at least body + descendants"
    );
    // DFS starts at body, so body_id must be the first entry in node_ids
    assert_eq!(
        scene.node_ids.first().copied(),
        scene.body_id,
        "node_ids[0] must be body_id (DFS starts at body, parity with paint_document)"
    );
    // Fragments coverage: every id in node_ids has at least one fragment
    for id in &scene.node_ids {
        assert!(
            scene.fragments.get(id).is_some_and(|f| !f.is_empty()),
            "fragments must have at least one entry for each id in node_ids ({id:?})",
        );
    }
    // No @page margin, but the UA sheet keeps `body { margin: 8px }`.
    // The hello-world `<p>` has its top margin zeroed by the quirks-mode
    // collapsing quirk (HTML LS section 15.3.9), so the vertical collapse is
    // max(0, 8, 0) = 8 and the horizontal offset is the plain 8px sum.
    assert_eq!(scene.body_offset_pt, (8.0, 8.0));
    // Page metadata reflects A4
    assert_eq!(
        scene.page_metadata.size,
        (PageBox::A4.width, PageBox::A4.height)
    );
    assert_eq!(scene.page_metadata.orientation, Orientation::Portrait);
}

/// Check that build_page_scene populates `drawables` with Element node →
/// `BlockEntry` and Text node → `ParagraphEntry` mappings. Regression
/// check: it also verifies that the production path exercises the
/// non-test `TrackedMap::insert` call site.
#[test]
fn build_page_scene_consumes_cascaded_margin_box_rules() {
    let opts = ParseOptions {
        extra_stylesheets: &[],
        network: None,
        base_url: None,
    };
    let uncascaded = parse(
        &b"<html><head><style>@page { @top-left { content: \"A\" } }</style></head><body>Hi</body></html>"[..],
        &opts,
    )
    .expect("parse Ok");
    let cascade = build_cascaded(&uncascaded);
    let mut dom = uncascaded.dom;
    raikiri_dom::layout_single_page(&mut dom, &cascade, PageBox::A4, FontContext::new())
        .expect("layout Ok");
    let scene = build_page_scene(&dom, &cascade, PageBox::A4);

    assert_eq!(scene.margin_boxes.len(), 1);
    assert_eq!(
        scene.margin_boxes[0].slot,
        raikiri_style::PageMarginBoxSlot::TopLeft
    );
    assert_eq!(scene.margin_boxes[0].declarations.len(), 1);
}

#[test]
fn build_page_scene_populates_block_and_paragraph_entries_from_hello_world() {
    let (dom, cascade) = hello_world_post_layout();
    let scene = build_page_scene(&dom, &cascade, PageBox::A4);

    // Every Element NodeId in node_ids has a block_styles entry, every
    // Text NodeId has a paragraphs entry — coverage must exactly match
    // node_ids (the same set as fragments; do not let them drift).
    for id in &scene.node_ids {
        let node = dom
            .get_node(id.0 as usize)
            .expect("node_ids entries resolve");
        match node.kind() {
            NodeKind::Element => {
                assert!(
                    scene.drawables.block_styles.contains_key(id),
                    "Element {id:?} must have a block_styles entry"
                );
            }
            NodeKind::Text => {
                assert!(
                    scene.drawables.paragraphs.contains_key(id),
                    "Text {id:?} must have a paragraphs entry"
                );
            }
            _ => {}
        }
    }

    // body_id resolves to an Element and must carry a BlockEntry with a
    // real (non-placeholder) layout_size — proves the cascade/layout
    // param is actually threaded, not just structurally accepted.
    let body_id = scene.body_id.expect("hello-world has <body>");
    let body_entry = scene
        .drawables
        .block_styles
        .get(&body_id)
        .expect("body must have a BlockEntry");
    assert!(
        body_entry.layout_size.is_some(),
        "BlockEntry.layout_size must be populated from post-layout Node.unrounded_layout"
    );
    // Gap fields hold the documented CSS-initial-value placeholders
    // (entries.rs module doc: opacity/visibility properties don't exist
    // in ComputedValues yet).
    assert_eq!(body_entry.opacity, 1.0);
    assert!(body_entry.visible);

    // At least one paragraph entry must have shaped lines (the "Hi" text
    // node) — proves text_layout() is actually read, not defaulted.
    assert!(
        scene
            .drawables
            .paragraphs
            .values()
            .any(|p| p.line_count > 0),
        "at least one ParagraphEntry must have line_count > 0 for shaped \"Hi\" text"
    );
}

/// Body UA margin reaches page-absolute geometry.
///
/// CSS 2.1 section 8.3.1 never collapses horizontal margins, so a child with
/// `margin: 10px` inside the UA `body { margin: 8px }` sits at 8 + 10 = 18
/// from the initial containing block. Vertically the same pair collapses to
/// max(8, 10) = 10. A plain block with no margin sits at 8 on both axes,
/// which is the `data-offset-x=8` shape that WPT check-layout fixtures pin.
#[test]
fn body_margin_horizontal_sum_and_vertical_collapse() {
    for (html, expected_offset, expected_frag, expected_abs) in [
        (
            "<div id=o style='margin:10px; width:10px; height:10px'></div>",
            (8.0, 0.0),
            (10.0, 10.0),
            (18.0, 10.0),
        ),
        (
            "<div id=b style='width:10px; height:10px'></div>",
            (8.0, 8.0),
            (0.0, 0.0),
            (8.0, 8.0),
        ),
    ] {
        let opts = ParseOptions {
            extra_stylesheets: &[],
            network: None,
            base_url: None,
        };
        let uncascaded = parse(html.as_bytes(), &opts).expect("parse Ok");
        let cascade = build_cascaded(&uncascaded);
        let mut dom = uncascaded.dom;
        raikiri_dom::layout_single_page(&mut dom, &cascade, PageBox::A4, FontContext::new())
            .expect("layout Ok");
        let scene = build_page_scene(&dom, &cascade, PageBox::A4);
        assert_eq!(
            scene.body_offset_pt, expected_offset,
            "body_offset for {html:?}"
        );
        let id = if html.contains("id=o") { "o" } else { "b" };
        let idx = (0..dom.node_count())
            .find(|&i| dom.element_attribute(i, "id") == Some(id))
            .expect("probe element resolves");
        let frag = scene
            .fragments
            .get(&NodeId::new(idx as u64))
            .and_then(|v| v.first())
            .expect("probe has a fragment");
        assert_eq!((frag.x, frag.y), expected_frag, "fragment for {html:?}");
        assert_eq!(
            (
                frag.x + scene.body_offset_pt.0,
                frag.y + scene.body_offset_pt.1
            ),
            expected_abs,
            "page-absolute geometry for {html:?}"
        );
    }
}

/// A lone `margin: 5px` block collapses with the UA body margin.
///
/// Horizontally the result is the sum 8 + 5 = 13. Vertically the adjoining
/// margins collapse to max(8, 5) = 8, so the page-absolute top is 8 even
/// though the body-relative fragment keeps its own 5px margin.
#[test]
fn body_margin_lone_5px_collapses_to_8px() {
    let opts = ParseOptions {
        extra_stylesheets: &[],
        network: None,
        base_url: None,
    };
    let html = "<div id=a style='margin:5px; width:10px; height:10px'></div>";
    let uncascaded = parse(html.as_bytes(), &opts).expect("parse Ok");
    let cascade = build_cascaded(&uncascaded);
    let mut dom = uncascaded.dom;
    raikiri_dom::layout_single_page(&mut dom, &cascade, PageBox::A4, FontContext::new())
        .expect("layout Ok");
    let scene = build_page_scene(&dom, &cascade, PageBox::A4);
    assert_eq!(scene.body_offset_pt, (8.0, 3.0));
    let idx = (0..dom.node_count())
        .find(|&i| dom.element_attribute(i, "id") == Some("a"))
        .expect("probe resolves");
    let frag = scene
        .fragments
        .get(&NodeId::new(idx as u64))
        .and_then(|v| v.first())
        .expect("probe has a fragment");
    assert_eq!((frag.x, frag.y), (5.0, 5.0));
    assert_eq!(
        (
            frag.x + scene.body_offset_pt.0,
            frag.y + scene.body_offset_pt.1
        ),
        (13.0, 8.0)
    );
}

/// Check that PageScene::rasterize returns the same PNG bytes as
/// html_to_png. This tests verbatim reuse of the byte-identical sequence
/// and pins the primary regression signal locally in this module.
#[test]
fn rasterize_matches_html_to_png_bytes() {
    let (dom, cascade) = hello_world_post_layout();
    let scene = build_page_scene(&dom, &cascade, PageBox::A4);
    let via_scene = scene.rasterize(&dom, &cascade, PageBox::A4);
    let via_umbrella = crate::html_to_png(&b"<p>Hi</p>"[..]).expect("html_to_png Ok");
    assert_eq!(
        via_scene, via_umbrella,
        "PageScene::rasterize must produce byte-identical output to html_to_png"
    );
}
