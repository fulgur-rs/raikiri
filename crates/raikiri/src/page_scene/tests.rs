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
    // Currently: no @page margin; body_offset_pt = (0, 0).
    assert_eq!(scene.body_offset_pt, (0.0, 0.0));
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
