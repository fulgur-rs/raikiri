//! The transforms of `raikiri_html::Page::paint_order` against where this
//! painter draws boxes and clips.
//!
//! Each fixture is laid out once through `raikiri_html::layout`. The corners
//! of every marked box and of every overflow clip in the paint order are
//! moved through the enclosing transform events, and the painter must fill
//! or clip a shape with those corners.

use anyrender::Scene;
use anyrender::recording::RenderCommand;
use kurbo::{Affine, BezPath, PathEl, Point};
use raikiri_html::{
    ClipKind, DocumentLayout, FontCollectionBuilder, LayoutConfig, LayoutOptions, LayoutStatus,
    Page, PageDefaults, PaintEvent, PaintRect, PaintTransform, RenderFonts, RenderResources,
    layout, parse_html_with_resources,
};
use raikiri_paint::paint_single_page_with_origin_and_page;

const AHEM: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../raikiri-dom/tests/data/text-autospace/Ahem.ttf"
));

const TOLERANCE: f64 = 1e-3;

fn fonts() -> RenderFonts {
    FontCollectionBuilder::new()
        .font_bytes("Ahem", AHEM.to_vec())
        .build()
        .expect("fonts")
}

fn lay_out(body: &str, css: &str) -> DocumentLayout {
    let html = format!(
        "<!doctype html><html><head><style>@page {{ size: 300px 200px; margin: 20px }} \
         body {{ font: 10px/10px Ahem }} .b {{ background: rgb(255, 0, 0) }} {css}\
         </style></head><body>{body}</body></html>"
    );
    let resources = RenderResources::new().fonts(fonts());
    let doc = parse_html_with_resources(html.as_bytes(), &resources).expect("parse");
    let status = layout(
        &doc,
        PageDefaults::default(),
        LayoutConfig::default(),
        LayoutOptions::new().resources(&resources),
    )
    .expect("layout");
    let LayoutStatus::Completed(result) = status else {
        panic!("expected a complete layout");
    };
    result
}

/// The vertices of `path` under `transform`.
fn vertices(transform: Affine, path: &BezPath) -> Vec<Point> {
    path.elements()
        .iter()
        .filter_map(|element| match *element {
            PathEl::MoveTo(point) | PathEl::LineTo(point) => Some(transform * point),
            _ => None,
        })
        .collect()
}

/// The vertices of the shapes the painter fills and of the clips it pushes
/// on `page`.
fn painted(page: &Page<'_>, page_count: u32) -> (Vec<Vec<Point>>, Vec<Vec<Point>>) {
    let (document, cascade, page_box, origin) = page.paint_inputs();
    let mut scene = Scene::new();
    paint_single_page_with_origin_and_page(
        &mut scene,
        document,
        cascade,
        page_box,
        origin,
        page.index(),
        page_count,
        page.index() % 2 == 1,
    )
    .expect("paint");
    let mut fills = Vec::new();
    let mut clips = Vec::new();
    for command in &scene.commands {
        match command {
            RenderCommand::Fill(fill) => {
                fills.push(vertices(fill.transform, &fill.shape));
            }
            RenderCommand::PushClipLayer(clip) => clips.push(vertices(clip.transform, &clip.clip)),
            _ => {}
        }
    }
    (fills, clips)
}

/// The corners of `rect` under `transform`.
fn corners(transform: PaintTransform, rect: PaintRect) -> Vec<Point> {
    [
        (rect.x, rect.y),
        (rect.x + rect.width, rect.y),
        (rect.x + rect.width, rect.y + rect.height),
        (rect.x, rect.y + rect.height),
    ]
    .into_iter()
    .map(|point| {
        let (x, y) = transform.apply(point);
        Point::new(f64::from(x), f64::from(y))
    })
    .collect()
}

/// Whether one of `shapes` has every point of `expected` as a vertex.
fn has_shape(shapes: &[Vec<Point>], expected: &[Point]) -> bool {
    shapes.iter().any(|shape| {
        expected.iter().all(|point| {
            shape
                .iter()
                .any(|vertex| vertex.distance(*point) < TOLERANCE)
        })
    })
}

/// Every marked box and every overflow clip of every page is where the
/// painter draws it. Returns how many were checked.
fn assert_matches_paint(body: &str, css: &str) -> usize {
    let result = lay_out(body, css);
    let mut checked = 0;
    for page in result.pages() {
        let (fills, clips) = painted(&page, result.page_count());
        let dom = page.dom();
        let mut transforms = vec![PaintTransform::IDENTITY];
        for event in page.paint_order() {
            let transform = *transforms.last().unwrap();
            match event {
                PaintEvent::PushTransform(own) => transforms.push(transform * own),
                PaintEvent::PopTransform => {
                    transforms.pop().expect("an open transform");
                }
                PaintEvent::Box(fragment) if dom.attr(fragment.node(), "class") == Some("b") => {
                    // The painter fills the whole box; a fragment shares the
                    // edges that the page does not cut.
                    let mut expected = corners(transform, fragment.paint_rect());
                    if fragment.is_last_fragment() == Some(false) {
                        expected.truncate(2);
                    }
                    if fragment.is_first_fragment() == Some(false) {
                        expected.drain(..2);
                    }
                    assert!(
                        has_shape(&fills, &expected),
                        "{body}: page {}: box {expected:?} is not filled; fills: {fills:?}",
                        page.index()
                    );
                    checked += 1;
                }
                // A generated box with the marked background, painted in
                // one piece.
                PaintEvent::GeneratedBox(piece)
                    if piece.style.background_color.r == 255
                        && piece.style.background_color.g == 0
                        && piece.has_start_edge
                        && piece.has_end_edge =>
                {
                    let expected = corners(transform, piece.rect);
                    assert!(
                        has_shape(&fills, &expected),
                        "{body}: page {}: generated box {expected:?} is not filled; fills: {fills:?}",
                        page.index()
                    );
                    checked += 1;
                }
                PaintEvent::PushClip(clip, ClipKind::Overflow) => {
                    let expected = corners(transform, clip.rect);
                    assert!(
                        has_shape(&clips, &expected),
                        "{body}: page {}: clip {expected:?} is not pushed; clips: {clips:?}",
                        page.index()
                    );
                    checked += 1;
                }
                _ => {}
            }
        }
        assert_eq!(transforms.len(), 1, "{body}: every transform is closed");
    }
    checked
}

#[test]
fn transformed_boxes_match_paint() {
    for body in [
        "<div class=\"b\" style=\"height: 20px; transform: rotate(30deg)\"></div>",
        "<div class=\"b\" style=\"height: 20px; transform: translate(7px, 3px)\"></div>",
        "<div class=\"b\" style=\"height: 20px; transform: scale(0.5, 2); transform-origin: 0 0\"></div>",
        "<div class=\"b\" style=\"height: 20px; transform: skew(10deg, 5deg)\"></div>",
        "<div class=\"b\" style=\"height: 20px; transform: matrix(0.8, 0.3, -0.2, 1.1, 7, 3)\"></div>",
        // The inner box turns about itself after the outer offsets.
        "<div class=\"b\" style=\"height: 60px; transform: translate(4px, 6px)\">\
         <div class=\"b\" style=\"position: relative; left: 3px; height: 40px; transform: rotate(-20deg)\">\
         <div class=\"b\" style=\"float: left; position: relative; top: 2px; width: 30px; height: 20px; \
         transform: rotate(45deg) scale(0.5)\"></div></div></div>",
        // Floats and table parts are moved by their relative offsets at
        // paint time.
        "<div class=\"b\" style=\"float: left; position: relative; left: 5px; top: 4px; width: 40px; \
         height: 20px\"></div>",
        "<table><tr><td class=\"b\" style=\"position: relative; left: 6px; top: 2px\">cell</td></tr></table>",
        // Inline elements that layout blockifies, and form controls, are
        // transformable.
        "<span class=\"b\" style=\"float: left; width: 30px; height: 20px; transform: rotate(20deg)\"></span>",
        "<span class=\"b\" style=\"position: absolute; width: 30px; height: 20px; \
         transform: translate(5px, 3px)\"></span>",
        "<button class=\"b\" style=\"display: inline; width: 30px; height: 20px; border: 0; padding: 0; \
         transform: rotate(15deg)\"></button>",
        // A box broken across pages turns about its box on each page.
        "<div class=\"b\" style=\"height: 300px; transform: rotate(2deg)\"></div>",
        // Percentages resolve against the box, in any function of the list.
        "<div class=\"b\" style=\"height: 20px; transform: translate(10%, 5px) rotate(10deg) \
         translateX(3px) translateY(25%) scaleX(0.9) scaleY(1.1)\"></div>",
        // A box above the page still turns the descendants that reach it.
        "<div style=\"position: absolute; top: -100px; left: 20px; width: 60px; height: 50px; \
         transform: rotate(30deg)\"><div class=\"b\" style=\"position: absolute; top: 120px; \
         width: 20px; height: 20px\"></div></div>",
        // A table header turns about its own copy on each page.
        "<table><thead><tr><th class=\"b\" style=\"transform: rotate(10deg)\">head</th></tr></thead>\
         <tbody><tr><td style=\"height: 250px\">body</td></tr></tbody></table>",
    ] {
        assert!(assert_matches_paint(body, "") > 0, "{body}");
    }
}

#[test]
fn transformed_pseudo_elements_match_paint() {
    for (body, css) in [
        (
            "<p>text</p>",
            "p::before { content: 'x'; display: inline-block; background: rgb(255, 0, 0); \
             transform: rotate(30deg) }",
        ),
        (
            "<div style=\"transform: translate(4px, 3px) rotate(-10deg)\"><p>text</p></div>",
            "p::after { content: 'x'; display: inline-block; width: 20px; height: 15px; \
             position: relative; left: 3px; opacity: 0.5; background: rgb(255, 0, 0); \
             transform: scale(0.5, 1.5) skewX(10deg); transform-origin: 25% 0 }",
        ),
        // An image alone in `content` replaces an inline pseudo-element,
        // which makes it transformable.
        (
            "<p>text</p>",
            "p::before { content: url(\"data:image/svg+xml,%3Csvg%20xmlns=%22http://www.w3.org/2000/svg%22%20width=%2210%22%20height=%2210%22/%3E\"); \
             background: rgb(255, 0, 0); transform: rotate(15deg) }",
        ),
    ] {
        assert!(assert_matches_paint(body, css) > 0, "{body} {css}");
    }
}

#[test]
fn clips_inside_transforms_match_paint() {
    for body in [
        "<div style=\"overflow: hidden; height: 20px; transform: rotate(10deg)\"><p>clipped</p></div>",
        "<div style=\"transform: translate(5px, 2px)\"><div style=\"overflow: hidden; height: 20px; \
         transform: scale(1.5)\"><p>clipped</p></div></div>",
    ] {
        assert!(assert_matches_paint(body, "") > 0, "{body}");
    }
}

#[test]
fn transformed_bodies_match_paint() {
    for css in [
        "body { transform: rotate(3deg) }",
        "body { transform: translate(4px, 2px) scale(0.9) }",
    ] {
        assert!(
            assert_matches_paint("<div class=\"b\" style=\"height: 20px\"></div>", css) > 0,
            "{css}"
        );
    }
}

#[test]
fn boxes_with_singular_transforms_are_left_out() {
    let result = lay_out(
        "<div class=\"b\" style=\"height: 20px; transform: scale(0, 1)\"><p>gone</p></div>",
        "",
    );
    let page = result.pages().next().expect("page");
    let dom = page.dom();
    assert!(!page.paint_order().iter().any(|event| {
        matches!(event, PaintEvent::Box(fragment) if dom.attr(fragment.node(), "class") == Some("b"))
    }));
    // Only the page background is filled.
    let (fills, _) = painted(&page, result.page_count());
    assert_eq!(fills.len(), 1, "{fills:?}");
}

#[test]
fn boxes_inside_columns_keep_their_layout_location() {
    // The column clips are listed at the content they clip, so the paint
    // order does not move the boxes inside columns.
    let result = lay_out(
        "<div style=\"columns: 2; height: 40px\"><div class=\"b\" style=\"height: 10px; \
         transform: translate(3px, 2px)\"></div></div>",
        "",
    );
    let page = result.pages().next().expect("page");
    assert!(
        !page
            .paint_order()
            .iter()
            .any(|event| matches!(event, PaintEvent::PushTransform(_)))
    );
}

#[test]
fn boxes_broken_across_columns_keep_their_layout_location() {
    // Each column fragment resolves a percentage against itself, which the
    // paint order does not express.
    let result = lay_out(
        "<div style=\"columns: 2\"><div class=\"b\" style=\"transform: translateY(10%)\">\
         <p>a</p><p>b</p><p>c</p><p>d</p></div></div>",
        "",
    );
    let page = result.pages().next().expect("page");
    assert!(
        !page
            .paint_order()
            .iter()
            .any(|event| matches!(event, PaintEvent::PushTransform(_)))
    );
}

#[test]
fn fixed_boxes_leave_the_relative_offsets_around_them() {
    // The painter places a fixed box in the page area, apart from the
    // relative offset the float around it is painted with.
    let result = lay_out(
        "<div class=\"b\" style=\"float: left; position: relative; left: 5px; top: 4px; \
         width: 40px; height: 20px\"><div id=\"fixed\" style=\"position: fixed; left: 5px; \
         top: 5px; width: 10px; height: 10px; background: blue\"></div></div>",
        "",
    );
    let page = result.pages().next().expect("page");
    let dom = page.dom();
    let mut transforms = vec![PaintTransform::IDENTITY];
    let mut seen = 0;
    for event in page.paint_order() {
        let transform = *transforms.last().unwrap();
        match event {
            PaintEvent::PushTransform(own) => transforms.push(transform * own),
            PaintEvent::PopTransform => {
                transforms.pop().expect("an open transform");
            }
            PaintEvent::Box(fragment) if dom.attr(fragment.node(), "class") == Some("b") => {
                assert_eq!(transform, PaintTransform::translate(5.0, 4.0));
                seen += 1;
            }
            PaintEvent::Box(fragment) if dom.attr(fragment.node(), "id") == Some("fixed") => {
                assert_eq!(transform, PaintTransform::IDENTITY);
                seen += 1;
            }
            _ => {}
        }
    }
    assert_eq!(seen, 2);
}

#[test]
fn fixed_boxes_inside_a_fixed_containing_block_match_paint() {
    // Each ancestor is the containing block of the fixed box (CSS Transforms
    // 1 §3, CSS Will Change 1 §2, CSS Containment 2 §3.2), so layout places
    // the fixed box against it and the painter must not move it to the page
    // area.
    for ancestor in [
        "will-change: transform",
        "will-change: filter",
        "contain: layout",
        "filter: opacity(1)",
    ] {
        let body = format!(
            "<div style=\"margin: 50px 0 0 30px; width: 100px; height: 60px; {ancestor}\">\
             <div class=\"b\" style=\"position: fixed; left: 5px; top: 7px; width: 10px; \
             height: 10px\"></div></div>"
        );
        assert_eq!(assert_matches_paint(&body, ""), 1, "{body}");
    }
}

#[test]
fn fixed_boxes_ignore_containing_blocks_above_the_body() {
    // Layout looks for the containing block of a fixed box from the body
    // down, so the root element never contains it, and the painter places
    // the box in the page area apart from the body's relative offset.
    for root in ["will-change: transform", "contain: layout"] {
        let css = format!("html {{ {root} }} body {{ position: relative; top: 20px }}");
        let body = "<div class=\"b\" style=\"position: fixed; left: 5px; top: 7px; width: 10px; \
                    height: 10px\"></div>";
        assert_eq!(assert_matches_paint(body, &css), 1, "{css}");
    }
}
