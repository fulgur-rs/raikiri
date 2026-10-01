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

/// Direct unit coverage for the body-margin helpers.
///
/// These pin the used-value resolution and collapsing arithmetic that the
/// integration tests above exercise only through the UA 8px path, so that
/// patch coverage sees every new non-test line at least once.
#[test]
fn body_margin_helpers_cover_used_value_branches() {
    use raikiri_style::property::CalcLengthPercentage;
    use raikiri_style::resolve::{ComputedLengthPercentage, ComputedLengthPercentageOrAuto};

    assert_eq!(
        super::used_margin_px(ComputedLengthPercentageOrAuto::Px(8.0), 800.0),
        8.0
    );
    assert_eq!(
        super::used_margin_px(ComputedLengthPercentageOrAuto::Percent(10.0), 800.0),
        80.0
    );
    assert_eq!(
        super::used_margin_px(
            ComputedLengthPercentageOrAuto::Calc(CalcLengthPercentage {
                percent: 10.0,
                px: 5.0
            }),
            800.0
        ),
        85.0
    );
    assert_eq!(
        super::used_margin_px(ComputedLengthPercentageOrAuto::Auto, 800.0),
        0.0
    );
    assert_eq!(
        super::used_margin_px(ComputedLengthPercentageOrAuto::Px(f32::NAN), 800.0),
        0.0
    );
    assert_eq!(
        super::used_padding_px(ComputedLengthPercentage::Px(3.0), 800.0),
        3.0
    );
    assert_eq!(
        super::used_padding_px(ComputedLengthPercentage::Percent(10.0), 800.0),
        80.0
    );
    assert_eq!(super::collapse_margins(&[8.0, 10.0]), 10.0);
    assert_eq!(super::collapse_margins(&[8.0, -5.0]), 3.0);
    assert_eq!(super::collapse_margins(&[-2.0, -5.0]), -5.0);
    assert_eq!(super::collapse_margins(&[f32::NAN, 8.0]), 8.0);
}

/// Barriers and empty-body fallbacks for the vertical offset.
///
/// A body border breaks parent-first collapsing, an invalid index falls back
/// to no barrier, and a missing body yields a zero offset.
#[test]
fn body_margin_helpers_cover_barrier_and_empty_fallbacks() {
    let opts = ParseOptions {
        extra_stylesheets: &[],
        network: None,
        base_url: None,
    };
    let uncascaded = parse(
        &b"<style>body{border:1px solid}</style><div id=b></div>"[..],
        &opts,
    )
    .expect("parse Ok");
    let cascade = build_cascaded(&uncascaded);
    let mut dom = uncascaded.dom;
    raikiri_dom::layout_single_page(&mut dom, &cascade, PageBox::A4, FontContext::new())
        .expect("layout Ok");
    let body_idx = (0..dom.node_count())
        .find(|&i| dom.get_node(i).and_then(|n| n.tag_name()) == Some("body"))
        .expect("body resolves");
    assert!(super::has_top_barrier(&cascade, body_idx, 800.0));
    assert!(!super::has_top_barrier(&cascade, usize::MAX, 800.0));
    assert_eq!(
        super::body_margin_offsets(&dom, &cascade, None, None, 800.0),
        (0.0, 0.0)
    );
    assert_eq!(
        super::body_margin_offsets(&dom, &cascade, Some(body_idx), None, 0.0).1,
        super::collapse_margins(&[super::used_margin_px(
            cascade.computed[body_idx].margin.top,
            0.0
        )])
    );
}

/// First-child blockers disable vertical collapsing.
///
/// Non-visible overflow, `display: none`, absolute positioning, floats, and a
/// leading text run all prevent the body top from collapsing with a child
/// margin, so the offset falls back to the `<html>`/`<body>` collapsed value.
#[test]
fn body_margin_helpers_cover_first_child_blockers() {
    for html in [
        "<style>body{overflow:hidden}</style><div id=b style='margin-top:10px'></div>",
        "<div id=b style='display:none'></div><div id=c></div>",
        "<div id=b style='position:absolute; margin-top:10px'></div>",
        "<div id=b style='float:left; margin-top:10px'></div>",
        "hello<div id=b style='margin-top:10px'></div>",
        "<!-- lead --><div id=b style='margin-top:10px'></div>",
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
        assert!(
            scene.body_offset_pt.0.is_finite(),
            "left finite for {html:?}"
        );
        assert!(
            scene.body_offset_pt.1.is_finite(),
            "top finite for {html:?}"
        );
    }
    let opts = ParseOptions {
        extra_stylesheets: &[],
        network: None,
        base_url: None,
    };
    let uncascaded = parse(&b"<div></div>"[..], &opts).expect("parse Ok");
    let cascade = build_cascaded(&uncascaded);
    let dom = uncascaded.dom;
    assert!(super::first_in_flow_top_margin(&dom, &cascade, usize::MAX, 800.0).is_none());
}

/// Defensive computed-missing and display-mismatch fallbacks.
///
/// A truncated cascade (fewer computed entries than DOM nodes) skips the
/// child, and a computed `display: none` that disagrees with the bridged
/// style also skips it. Both are caller-responsibility violations that the
/// scene treats as no adjoining margin.
#[test]
fn body_margin_helpers_cover_computed_fallbacks() {
    let opts = ParseOptions {
        extra_stylesheets: &[],
        network: None,
        base_url: None,
    };
    let uncascaded =
        parse(&b"<div id=b style='margin-top:10px'></div>"[..], &opts).expect("parse Ok");
    let mut cascade = build_cascaded(&uncascaded);
    let mut dom = uncascaded.dom;
    raikiri_dom::layout_single_page(&mut dom, &cascade, PageBox::A4, FontContext::new())
        .expect("layout Ok");
    let body_idx = (0..dom.node_count())
        .find(|&i| dom.get_node(i).and_then(|n| n.tag_name()) == Some("body"))
        .expect("body resolves");
    let empty_uncascaded = parse(&b""[..], &opts).expect("parse Ok");
    let empty_cascade = build_cascaded(&empty_uncascaded);
    assert!(super::first_in_flow_top_margin(&dom, &empty_cascade, body_idx, 800.0).is_none());
    let child_idx = (0..dom.node_count())
        .find(|&i| dom.element_attribute(i, "id") == Some("b"))
        .expect("probe resolves");
    cascade.computed[child_idx].display = raikiri_style::property::DisplayValue::None;
    assert!(super::first_in_flow_top_margin(&dom, &cascade, body_idx, 800.0).is_none());
}

/// `<html>` barriers fall back to the plain sum.
///
/// An `<html>` top border disables `<html>`/`<body>` collapsing per CSS 2.1
/// section 8.3.1; the scene keeps the simple sum as an approximation.
#[test]
fn body_margin_helpers_cover_html_barrier_fallback() {
    let opts = ParseOptions {
        extra_stylesheets: &[],
        network: None,
        base_url: None,
    };
    let uncascaded = parse(
        &b"<style>html{border:2px solid} body{margin-top:8px}</style><div id=b></div>"[..],
        &opts,
    )
    .expect("parse Ok");
    let cascade = build_cascaded(&uncascaded);
    let mut dom = uncascaded.dom;
    raikiri_dom::layout_single_page(&mut dom, &cascade, PageBox::A4, FontContext::new())
        .expect("layout Ok");
    let scene = build_page_scene(&dom, &cascade, PageBox::A4);
    assert!(scene.body_offset_pt.0.is_finite());
    assert!(scene.body_offset_pt.1.is_finite());
}


const FONT_DIR: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../raikiri-dom/tests/data/text-autospace"
);

/// Parse the HTML, switch the inline engine on, lay it out on one A4 page,
/// and build the page scene.
fn scene_with_engine(html: &str) -> (Document, CascadeResult, PageScene) {
    let opts = ParseOptions {
        extra_stylesheets: &[],
        network: None,
        base_url: None,
    };
    let uncascaded = parse(html.as_bytes(), &opts).expect("parse Ok");
    let cascade = build_cascaded(&uncascaded);
    let mut dom = uncascaded.dom;
    let dir = std::path::Path::new(FONT_DIR);
    dom.enable_inline_formatting(
        raikiri_dom::build_wpt_font_collection(dir).expect("collection"),
        shodo::limits::Limits::default(),
    );
    let fonts = raikiri_dom::build_wpt_font_ctx(dir).expect("font context");
    raikiri_dom::layout_single_page(&mut dom, &cascade, PageBox::A4, fonts).expect("layout Ok");
    let scene = build_page_scene(&dom, &cascade, PageBox::A4);
    (dom, cascade, scene)
}

fn node(dom: &Document, id: usize) -> &raikiri_dom::Node {
    dom.get_node(id).expect("node")
}

/// The first in-document element named `tag`.
fn find_element(dom: &Document, tag: &str) -> usize {
    (0..dom.node_count())
        .find(|&id| {
            dom.get_node(id)
                .is_some_and(|node| node.is_in_document() && node.tag_name() == Some(tag))
        })
        .expect("element")
}

fn rects(scene: &PageScene, id: usize) -> Vec<(f32, f32, f32, f32)> {
    scene.fragments[&NodeId::new(id as u64)]
        .iter()
        .map(|f| (f.x, f.y, f.width, f.height))
        .collect()
}

/// `aa <span>bb cc</span>` in a 40px Ahem 10px block: the lines are "aa",
/// "bb" and "cc".
const WRAPPING_SPAN: &str = "<style>body{margin:0} div{font:10px/10px Ahem;width:40px}</style>\
     <div>aa <span>bb cc</span></div>";

#[test]
fn an_ifc_text_reports_its_lines_and_a_wrapping_span_one_fragment_per_line() {
    let (dom, _cascade, scene) = scene_with_engine(WRAPPING_SPAN);
    let div = find_element(&dom, "div");
    let span = find_element(&dom, "span");
    assert!(
        node(&dom, div).is_ifc_root(),
        "the paragraph is laid out by the engine"
    );
    let first = node(&dom, div).children[0]; // "aa "
    let inner = node(&dom, span).children[0]; // "bb cc"
    assert_eq!(rects(&scene, first), [(0.0, 0.0, 40.0, 10.0)]);
    assert_eq!(rects(&scene, inner), [(0.0, 10.0, 40.0, 20.0)]);
    assert_eq!(
        rects(&scene, span),
        [(0.0, 10.0, 20.0, 10.0), (0.0, 20.0, 20.0, 10.0)]
    );
    let line_count = |id: usize| scene.drawables.paragraphs[&NodeId::new(id as u64)].line_count;
    assert_eq!((line_count(first), line_count(inner)), (1, 2));
}

#[test]
fn an_ifc_text_starts_at_the_padded_root_content_origin() {
    let (dom, _cascade, scene) = scene_with_engine(
        "<style>body{margin:0} div{font:10px/10px Ahem;width:40px;padding:5px}</style>\
         <div>aa <span>bb cc</span></div>",
    );
    let div = find_element(&dom, "div");
    let span = find_element(&dom, "span");
    let inner = node(&dom, span).children[0];
    assert_eq!(rects(&scene, inner), [(5.0, 15.0, 40.0, 20.0)]);
    assert_eq!(
        rects(&scene, span),
        [(5.0, 15.0, 20.0, 10.0), (5.0, 25.0, 20.0, 10.0)]
    );
    assert!(node(&dom, div).is_ifc_root());
}

#[test]
fn each_inline_element_gets_only_its_own_pieces() {
    let (dom, _cascade, scene) = scene_with_engine(
        "<style>body{margin:0} div{font:10px/10px Ahem;width:80px}</style>\
         <div><span>aa</span> <em>bb</em></div>",
    );
    let div = find_element(&dom, "div");
    assert!(node(&dom, div).is_ifc_root());
    assert_eq!(
        rects(&scene, find_element(&dom, "span")),
        [(0.0, 0.0, 20.0, 10.0)]
    );
    assert_eq!(
        rects(&scene, find_element(&dom, "em")),
        [(30.0, 0.0, 20.0, 10.0)]
    );
}

#[test]
fn a_later_page_gets_only_the_lines_and_pieces_below_its_top() {
    let (dom, cascade, _scene) = scene_with_engine(WRAPPING_SPAN);
    // A page whose content starts at y 20: the "cc" line (20..30) is on it,
    // the "aa" (0..10) and "bb" (10..20) lines are not.
    let scene = build_page_scene_for_page(&dom, &cascade, PageBox::A4, 1, 20.0);
    let div = find_element(&dom, "div");
    let span = find_element(&dom, "span");
    let first = node(&dom, div).children[0];
    let inner = node(&dom, span).children[0];
    assert!(!scene.fragments.contains_key(&NodeId::new(first as u64)));
    assert_eq!(rects(&scene, inner), [(0.0, -10.0, 40.0, 20.0)]);
    assert_eq!(rects(&scene, span), [(0.0, 0.0, 20.0, 10.0)]);
}

/// Paginating a tall document splits nodes across per-page scenes.
///
/// A short `<div id=top>` above a 2000px spacer fits on page 0 but lies
/// fully above page 1's interval, while `<div id=low>` below the spacer
/// only intersects page 1. This pins the page-intersection gate in
/// [`build_page_scene_for_page`](super::build_page_scene_for_page) both
/// ways (include + exclude) and the body-on-every-page contract, plus
/// fragment-y rebasing by `content_origin_y` and the `page_index` /
/// `content_origin_y` plumbing that the single-page helper leaves at zero.
#[test]
fn build_page_scene_for_page_splits_tall_content_across_pages() {
    let opts = ParseOptions {
        extra_stylesheets: &[],
        network: None,
        base_url: None,
    };
    let html = concat!(
        "<div id=top style='height:10px'></div>",
        "<div style='height:2000px'></div>",
        "<div id=low style='height:10px'></div>",
    );
    let uncascaded = parse(html.as_bytes(), &opts).expect("parse Ok");
    let cascade = build_cascaded(&uncascaded);
    let mut dom = uncascaded.dom;
    raikiri_dom::layout_single_page(&mut dom, &cascade, PageBox::A4, FontContext::new())
        .expect("layout Ok");

    // Derive page 1's origin from the same public page-geometry API the
    // scene builder uses, so the test tracks geometry changes instead of
    // hard-coding A4 content height.
    let margins = raikiri_dom::page_margins(&cascade, PageBox::A4);
    let insets = raikiri_dom::page_content_insets(&cascade, PageBox::A4);
    let content_height =
        (margins.content_height(PageBox::A4) - insets.top - insets.bottom).max(0.0);
    assert!(
        18.0 < content_height && content_height < 2000.0,
        "page 0 must fully contain #top yet end above #low; got {content_height}"
    );

    let page0 = build_page_scene_for_page(&dom, &cascade, PageBox::A4, 0, 0.0);
    let page1 = build_page_scene_for_page(&dom, &cascade, PageBox::A4, 1, content_height);

    let top_id = NodeId::new(
        (0..dom.node_count())
            .find(|&i| dom.element_attribute(i, "id") == Some("top"))
            .expect("probe #top resolves") as u64,
    );
    let low_id = NodeId::new(
        (0..dom.node_count())
            .find(|&i| dom.element_attribute(i, "id") == Some("low"))
            .expect("probe #low resolves") as u64,
    );

    assert!(page0.node_ids.contains(&top_id), "#top fits on page 0");
    assert!(!page0.node_ids.contains(&low_id), "#low is below page 0");
    assert!(!page1.node_ids.contains(&top_id), "#top is above page 1");
    assert!(page1.node_ids.contains(&low_id), "#low fits on page 1");

    // The body container is present on every page so its propagated
    // background stays available; both scenes start their DFS there.
    let body_id = page0.body_id.expect("shaped doc has <body>");
    assert_eq!(page0.node_ids.first(), Some(&body_id));
    assert_eq!(page1.node_ids.first(), Some(&body_id));

    // Fragment y is rebased into page-local space; adding the origin back
    // recovers document order across the page boundary.
    let top_y_page0 = page0
        .fragments
        .get(&top_id)
        .and_then(|v| v.first())
        .expect("#top has a fragment on page 0")
        .y;
    let low_y_page1 = page1
        .fragments
        .get(&low_id)
        .and_then(|v| v.first())
        .expect("#low has a fragment on page 1")
        .y;
    assert!(
        low_y_page1 >= 0.0,
        "page-local y is non-negative; got {low_y_page1}"
    );
    assert!(
        top_y_page0 + page0.content_origin_y < low_y_page1 + page1.content_origin_y,
        "document order survives rebasing"
    );
    assert_eq!(page0.page_metadata.page_index, 0);
    assert_eq!(page1.page_metadata.page_index, 1);
    assert_eq!(page1.content_origin_y, content_height);
}

/// The named page-scene entry attaches the pagination pass's page name.
///
/// A `size: 300px 50px` descriptor also pins landscape orientation through
/// this entry point (width > height), complementing the portrait A4 scenes
/// elsewhere. `content_origin_y` stays zero for a first-page extraction.
#[test]
fn build_page_scene_for_page_named_attaches_name_and_landscape() {
    let opts = ParseOptions {
        extra_stylesheets: &[],
        network: None,
        base_url: None,
    };
    let uncascaded = parse(
        &b"<html><head><style>@page { size: 300px 50px }</style></head><body>Hi</body></html>"[..],
        &opts,
    )
    .expect("parse Ok");
    let cascade = build_cascaded(&uncascaded);
    let mut dom = uncascaded.dom;
    let page_box = PageBox::from_page_size(cascade.page.size());
    raikiri_dom::layout_single_page(&mut dom, &cascade, page_box, FontContext::new())
        .expect("layout Ok");
    let scene = build_page_scene_for_page_named(
        &dom,
        &cascade,
        page_box,
        2,
        0.0,
        Some("first".to_string()),
    );
    assert_eq!(scene.page_metadata.size, (300.0, 50.0));
    assert_eq!(scene.page_metadata.orientation, Orientation::Landscape);
    assert_eq!(scene.page_metadata.page_index, 2);
    assert_eq!(scene.page_metadata.page_name.as_deref(), Some("first"));
    assert_eq!(scene.content_origin_y, 0.0);
    assert!(
        !scene.node_ids.is_empty(),
        "named scene still extracts the body subtree"
    );
}

/// `encode_png` rejects a buffer whose length disagrees with the canvas.
///
/// This pins the byte-count invariant message (the only `encode_png`
/// failure reachable without a broken renderer) instead of leaving the
/// assert-format lines uncovered.
#[test]
#[should_panic(expected = "encode_png: expected")]
fn encode_png_rejects_mismatched_buffer_length() {
    super::encode_png(&[0u8; 3], 1, 1);
}
