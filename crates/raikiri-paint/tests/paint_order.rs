//! `raikiri_html::Page::paint_order` against the order this painter walks
//! the body in.
//!
//! Each fixture is laid out once through `raikiri_html::layout`; for every
//! page the body walk is traced with `trace_paint_order` and both sequences
//! are reduced to a common shape before they are compared.

use raikiri_dom::{CounterSnapshotBudget, Document};
use raikiri_html::{
    ClipKind, DocumentLayout, FontCollectionBuilder, FragmentKind, LayoutConfig, LayoutOptions,
    LayoutStatus, NodeKind, Page, PageDefaults, PaintEvent, RenderFonts, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_paint::{PaintTraceEvent, trace_paint_order};

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

/// A small page so a few paragraphs overflow onto later pages.
const PAGE: &str = "@page { size: 300px 200px; margin: 20px }";

fn lay_out(body: &str, css: &str) -> DocumentLayout {
    let html = format!(
        "<!doctype html><html><head><style>{PAGE} \
         body {{ font: 10px/10px Ahem }} {css}</style></head><body>{body}</body></html>"
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

/// The common shape both sequences are reduced to. Clips and opacity groups
/// carry no node: the public events do not name the element they belong to.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Step {
    PushClip,
    PopClip,
    PushOpacity,
    PopOpacity,
    Box(usize),
    Replaced(usize),
    Text(usize),
}

fn trace(page: &Page<'_>) -> Vec<PaintTraceEvent> {
    let (document, cascade, page_box, origin) = page.paint_inputs();
    let mut budget = CounterSnapshotBudget::default();
    trace_paint_order(
        document,
        cascade,
        page_box,
        origin,
        Some(page.name()),
        &mut budget,
    )
    .expect("trace")
}

/// The walker's steps, without column clips (the public list does not have
/// them yet) and without the pops that close them.
///
/// An overflow clip that lies wholly outside the page hides everything
/// inside it, so such a clip and the steps it encloses are left out: the
/// walker still visits that subtree, but draws nothing of it on the page.
fn walker_steps(page: &Page<'_>) -> Vec<Step> {
    let page_box = page.geometry().page_box;
    let (page_w, page_h) = (f64::from(page_box.width), f64::from(page_box.height));
    let off_page =
        |[x0, y0, x1, y1]: [f64; 4]| x1 <= 0.0 || y1 <= 0.0 || x0 >= page_w || y0 >= page_h;
    let mut steps = Vec::new();
    // Each open clip: whether it is a column clip.
    let mut clips: Vec<bool> = Vec::new();
    // How many of the open clips lie wholly outside the page.
    let mut hidden = 0usize;
    // The depth of `clips` at which each off-page clip was opened.
    let mut hidden_at: Vec<usize> = Vec::new();
    for event in trace(page) {
        match event {
            PaintTraceEvent::PushOverflowClip(_, rect) => {
                clips.push(false);
                if off_page(rect) {
                    hidden += 1;
                    hidden_at.push(clips.len());
                } else if hidden == 0 {
                    steps.push(Step::PushClip);
                }
            }
            PaintTraceEvent::PushFragmentainerClip(_) => clips.push(true),
            PaintTraceEvent::PopClip => {
                if hidden_at.last() == Some(&clips.len()) {
                    hidden_at.pop();
                    hidden -= 1;
                    clips.pop();
                    continue;
                }
                if !clips.pop().expect("a clip to close") && hidden == 0 {
                    steps.push(Step::PopClip);
                }
            }
            _ if hidden > 0 => {}
            PaintTraceEvent::PushOpacity(..) => steps.push(Step::PushOpacity),
            PaintTraceEvent::PopOpacity => steps.push(Step::PopOpacity),
            PaintTraceEvent::Box(node) => steps.push(Step::Box(node)),
            PaintTraceEvent::Replaced(node) => steps.push(Step::Replaced(node)),
            PaintTraceEvent::Text(node) => steps.push(Step::Text(node)),
        }
    }
    steps
}

/// The paragraph a text node's lines belong to: the node itself when it is
/// a paragraph root, else its nearest paragraph root ancestor.
fn paragraph_root(document: &Document, node: usize) -> usize {
    std::iter::successors(Some(node), |&id| document.parent_of(id))
        .find(|&id| document.get_node(id).is_some_and(|n| n.is_ifc_root()))
        .unwrap_or(node)
}

fn node_index(id: raikiri_html::NodeId) -> usize {
    usize::try_from(id.0).expect("node id")
}

/// The public list's steps. Inline element boxes are dropped (the walker
/// draws them with the lines and does not record them), and the text of one
/// paragraph is one step, as the walker draws a paragraph's lines at once.
fn api_steps(page: &Page<'_>) -> Vec<Step> {
    let (document, _, _, _) = page.paint_inputs();
    let mut steps = Vec::new();
    for event in page.paint_order() {
        let step = match event {
            PaintEvent::PushClip(_, ClipKind::Fragmentainer) => {
                panic!("column clips are not listed yet")
            }
            PaintEvent::PushClip(..) => Step::PushClip,
            PaintEvent::PopClip => Step::PopClip,
            PaintEvent::PushOpacity(_) => Step::PushOpacity,
            PaintEvent::PopOpacity => Step::PopOpacity,
            PaintEvent::Box(fragment) => {
                let node = node_index(fragment.node());
                if document
                    .get_node(node)
                    .is_some_and(|n| n.kind() == NodeKind::Element && n.in_ifc_subtree())
                {
                    continue;
                }
                Step::Box(node)
            }
            PaintEvent::Replaced(fragment) => Step::Replaced(node_index(fragment.node())),
            PaintEvent::Text(fragment) => {
                Step::Text(paragraph_root(document, node_index(fragment.node())))
            }
            _ => panic!("unexpected event {event:?}"),
        };
        if matches!(step, Step::Text(_)) && steps.last() == Some(&step) {
            continue;
        }
        steps.push(step);
    }
    steps
}

/// Names the node of each step so a failure message is readable.
fn describe(page: &Page<'_>, steps: &[Step]) -> String {
    let (document, _, _, _) = page.paint_inputs();
    let name = |node: usize| {
        document.get_node(node).map_or_else(
            || "?".to_owned(),
            |n| match (n.tag_name(), n.attribute("id")) {
                (Some(tag), Some(id)) => format!("<{tag} id={id}>"),
                (Some(tag), None) => format!("<{tag}>"),
                _ => format!("{:?}", n.kind()),
            },
        )
    };
    steps
        .iter()
        .map(|step| match *step {
            Step::Box(n) => format!("  Box({n}) {}", name(n)),
            Step::Replaced(n) => format!("  Replaced({n}) {}", name(n)),
            Step::Text(n) => format!("  Text({n}) {}", name(n)),
            other => format!("  {other:?}"),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The node of the element with `id`.
fn by_id(result: &DocumentLayout, id: &str) -> usize {
    let page = result.pages().next().expect("a page");
    let (document, _, _, _) = page.paint_inputs();
    (0..document.node_count())
        .find(|&node| {
            document
                .get_node(node)
                .is_some_and(|n| n.attribute("id") == Some(id))
        })
        .unwrap_or_else(|| panic!("no element with id {id}"))
}

/// The `<body>` element's node.
fn body_node(page: &Page<'_>) -> usize {
    let (document, _, _, _) = page.paint_inputs();
    (0..document.node_count())
        .find(|&node| {
            document
                .get_node(node)
                .is_some_and(|n| n.tag_name() == Some("body"))
        })
        .expect("a body element")
}

/// The walker's steps with the body's box taken out on pages where the
/// page projection has no fragment of the body: the walker records the
/// body's box on every page (see `known_difference_body_box_on_every_page`).
fn walker_steps_on_body_pages(page: &Page<'_>) -> Vec<Step> {
    let body = body_node(page);
    let mut walker = walker_steps(page);
    let body_on_page = page
        .fragments()
        .any(|f| node_index(f.node()) == body && f.kind() == FragmentKind::Box);
    if !body_on_page && walker.first() == Some(&Step::Box(body)) {
        walker.remove(0);
    }
    walker
}

/// Compares both sequences on every page and returns the layout so a
/// fixture can assert its own premises (how many pages, which page names).
fn assert_same_order(body: &str, css: &str) -> DocumentLayout {
    let result = lay_out(body, css);
    assert!(result.page_count() > 0, "no pages for {body}");
    let mut paints_a_box = false;
    for page in result.pages() {
        let api = api_steps(&page);
        let walker = walker_steps_on_body_pages(&page);
        paints_a_box |= walker.iter().any(|step| matches!(step, Step::Box(_)));
        assert!(
            walker == api,
            "page {} of {body}\nwalker:\n{}\npaint_order:\n{}",
            page.index(),
            describe(&page, &walker),
            describe(&page, &api),
        );
    }
    assert!(paints_a_box, "{body} paints no box");
    result
}

/// Asserts that a fixture meant to span pages does.
fn assert_multi_page(result: &DocumentLayout) {
    assert!(
        result.page_count() > 1,
        "expected several pages, got {}",
        result.page_count()
    );
}

/// The walker draws the body's box on every page, as the root of the page's
/// content; `paint_order` lists it only on the pages its box reaches. Here
/// the body's box ends on page 1 while its paragraphs go on to page 2, and
/// on the pages after the first the paragraphs alone have a box.
#[test]
fn known_difference_body_box_on_every_page() {
    let paragraphs: String = (0..20).map(|i| format!("<p>paragraph {i}</p>")).collect();
    let result = lay_out(
        &paragraphs,
        "body { border: 3px solid black; background: #eee }",
    );
    assert_eq!(result.page_count(), 3);
    for page in result.pages() {
        let body = body_node(&page);
        let walker = walker_steps(&page);
        let api = api_steps(&page);
        assert_eq!(
            walker.first(),
            Some(&Step::Box(body)),
            "page {}",
            page.index()
        );
        if page.index() == 2 {
            // The walker has the body's box first; paint_order has no body
            // box and is otherwise the same.
            assert!(!api.contains(&Step::Box(body)));
            assert_eq!(walker[1..], api[..], "page {}", page.index());
        } else {
            assert_eq!(walker, api, "page {}", page.index());
        }
    }
}

#[test]
fn paragraphs_over_several_pages() {
    let body: String = (0..30).map(|i| format!("<p>paragraph {i}</p>")).collect();
    let result = assert_same_order(&body, "");
    assert_multi_page(&result);
}

#[test]
fn absolute_after_a_paragraph() {
    assert_same_order(
        "<div style=\"position: relative\"><p>some text that wraps over a line or two</p>\
         <div style=\"position: absolute; top: 0; left: 0; width: 50px; height: 20px; \
         background: red\">abs</div></div>",
        "",
    );
}

#[test]
fn z_index_siblings() {
    assert_same_order(
        "<div style=\"position: relative\">\
         <div style=\"position: relative; z-index: 2; background: red\">two</div>\
         <div style=\"position: relative; z-index: -1; background: blue\">minus</div>\
         <div style=\"position: relative; background: green\">auto</div>\
         <div style=\"background: gray\">flow</div>\
         <div style=\"position: relative; z-index: 0; background: teal\">zero</div>\
         </div>",
        "",
    );
}

#[test]
fn flex_order_and_z_index() {
    assert_same_order(
        "<div style=\"display: flex\">\
         <div style=\"order: 2; background: red\">a</div>\
         <div style=\"order: -1; background: blue\">b</div>\
         <div style=\"position: relative; z-index: 1; order: -2; background: green\">c</div>\
         <div style=\"background: gray\">d</div>\
         </div>",
        "",
    );
}

#[test]
fn float_and_in_flow_siblings() {
    assert_same_order(
        "<div><div style=\"float: left; width: 40px; height: 40px; background: red\">f</div>\
         <div style=\"background: blue\">in flow block</div>\
         <p>text beside <span style=\"background: yellow\">the</span> float</p></div>",
        "",
    );
}

/// The walker draws an inline element's background with the paragraph's
/// lines and records only the paragraph's `Text`; `paint_order` lists the
/// inline element's `Box` on its own, before the paragraph's text. The
/// comparison above drops such boxes for that reason.
#[test]
fn known_difference_inline_boxes_listed_before_text() {
    let result = lay_out(
        "<p id=p>text beside <span id=s style=\"background: yellow\">the</span> float</p>",
        "",
    );
    let span = by_id(&result, "s");
    let paragraph = by_id(&result, "p");
    let page = result.pages().next().expect("a page");
    let (document, _, _, _) = page.paint_inputs();
    let walker = trace(&page);
    assert!(!walker.contains(&PaintTraceEvent::Box(span)));
    assert!(walker.contains(&PaintTraceEvent::Text(paragraph)));
    let events = page.paint_order();
    let span_box = events
        .iter()
        .position(|e| matches!(e, PaintEvent::Box(f) if node_index(f.node()) == span))
        .expect("the span's box is listed");
    let first_text = events
        .iter()
        .position(|e| {
            matches!(e, PaintEvent::Text(f)
                if paragraph_root(document, node_index(f.node())) == paragraph)
        })
        .expect("the paragraph's text is listed");
    assert!(span_box < first_text);
}

const NESTED_CLIPS: &str = "<div style=\"opacity: 0.5; background: red\">\
     <div style=\"overflow: hidden; height: 30px; padding: 3px; border: 2px solid black\">\
     <div style=\"opacity: 0.7; overflow: clip; width: 100px; height: 20px\">\
     <p>clipped text that goes on and on</p></div></div></div>";

#[test]
fn nested_opacity_and_overflow() {
    assert_same_order(NESTED_CLIPS, "");
}

#[test]
fn overflow_hidden_box_across_pages() {
    let inner: String = (0..25).map(|i| format!("<p>line {i}</p>")).collect();
    let result = assert_same_order(
        &format!("<div style=\"overflow: hidden; background: #eee\">{inner}</div>"),
        "",
    );
    assert_multi_page(&result);
}

#[test]
fn fixed_header_on_every_page() {
    let body: String = (0..30).map(|i| format!("<p>paragraph {i}</p>")).collect();
    let result = assert_same_order(
        &format!(
            "<div style=\"position: fixed; top: 0; left: 0; background: yellow\">header</div>\
             {body}"
        ),
        "",
    );
    assert_multi_page(&result);
}

#[test]
fn img_and_inline_svg() {
    assert_same_order(
        "<p>before <img src=\"data:image/png;base64,\" width=\"10\" height=\"10\" \
         style=\"background: red\"> \
         <svg width=\"10\" height=\"10\"><rect width=\"5\" height=\"5\"/></svg> after</p>\
         <img style=\"display: block; width: 20px; height: 20px\" src=\"missing.png\">",
        "",
    );
}

#[test]
fn visibility_hidden_table() {
    assert_same_order(
        "<table style=\"visibility: hidden; border: 1px solid; background: red\">\
         <tr><td style=\"visibility: visible; background: blue\">cell</td>\
         <td>hidden</td></tr></table>",
        "",
    );
}

#[test]
fn body_with_a_border() {
    let body: String = (0..20).map(|i| format!("<p>paragraph {i}</p>")).collect();
    let result = assert_same_order(&body, "body { border: 3px solid black; background: #eee }");
    assert_multi_page(&result);
}

#[test]
fn named_page() {
    let result = assert_same_order(
        "<p>first</p><div style=\"page: wide; background: red\">wide \
         <div style=\"float: left; width: 20px; height: 20px; background: blue\"></div>\
         text</div><p>last</p>",
        "@page wide { size: 400px 200px }",
    );
    assert!(
        result.pages().any(|page| page.name() == Some("wide")),
        "no page is named wide"
    );
}

#[test]
fn absolute_continuation() {
    let inner: String = (0..25).map(|i| format!("<p>line {i}</p>")).collect();
    let result = assert_same_order(
        &format!(
            "<div style=\"position: absolute; top: 0; left: 0; border: 2px solid; \
             background: #eee\">{inner}</div>"
        ),
        "",
    );
    assert_multi_page(&result);
}

/// An overflow box that starts on the first page and ends there while its
/// content runs on: on the later pages the box has no fragment, and the
/// walker's clip for it lies wholly above the page, so nothing inside it is
/// drawn there.
#[test]
fn overflow_box_absent_from_later_pages() {
    let inner: String = (0..25).map(|i| format!("<p>line {i}</p>")).collect();
    let result = assert_same_order(
        &format!(
            "<p>x</p><div style=\"overflow: hidden; height: 100px; background: #eee\">\
             {inner}</div><p>after</p>"
        ),
        "",
    );
    assert_multi_page(&result);
    // The premise: on a later page the walker opens the box's clip off the
    // page, and nothing of the box's content is left to compare.
    let page = result.pages().nth(1).expect("a second page");
    assert!(
        trace(&page)
            .iter()
            .any(|event| matches!(event, PaintTraceEvent::PushOverflowClip(..)))
    );
    assert!(api_steps(&page).is_empty(), "{:?}", api_steps(&page));
}

/// As above with an opacity group on the box: on the later pages the group
/// is still opened and closed, with nothing in it.
#[test]
fn translucent_overflow_box_absent_from_later_pages() {
    let inner: String = (0..25).map(|i| format!("<p>line {i}</p>")).collect();
    let result = assert_same_order(
        &format!(
            "<p>x</p><div style=\"opacity: 0.5; overflow: hidden; height: 100px\">\
             {inner}</div><p>after</p>"
        ),
        "",
    );
    assert_multi_page(&result);
    let page = result.pages().nth(1).expect("a second page");
    assert_eq!(api_steps(&page), [Step::PushOpacity, Step::PopOpacity]);
}

/// A paragraph root whose only line holds a canvas and whitespace: no
/// visible text, so neither list has the paragraph's text.
#[test]
fn inline_content_directly_in_body() {
    assert_same_order("<canvas width=10 height=10></canvas> ", "");
}

/// The walker's overflow clip rectangles on `page` and the public list's,
/// in order.
fn clip_rects(page: &Page<'_>) -> (Vec<[f64; 4]>, Vec<[f64; 4]>) {
    let walker: Vec<[f64; 4]> = trace(page)
        .into_iter()
        .filter_map(|event| match event {
            PaintTraceEvent::PushOverflowClip(_, rect) => Some(rect),
            _ => None,
        })
        .collect();
    let api: Vec<[f64; 4]> = page
        .paint_order()
        .into_iter()
        .filter_map(|event| match event {
            PaintEvent::PushClip(clip, ClipKind::Overflow) => {
                let r = clip.rect;
                Some([
                    f64::from(r.x),
                    f64::from(r.y),
                    f64::from(r.x + r.width),
                    f64::from(r.y + r.height),
                ])
            }
            _ => None,
        })
        .collect();
    (walker, api)
}

fn assert_rects_match(walker: &[[f64; 4]], api: &[[f64; 4]], context: &str) {
    assert_eq!(
        walker.len(),
        api.len(),
        "{context}: walker {walker:?}, paint_order {api:?}"
    );
    for (w, a) in walker.iter().zip(api) {
        for (x, y) in w.iter().zip(a) {
            assert!(
                (x - y).abs() < TOLERANCE,
                "{context}: walker {w:?}, paint_order {a:?}"
            );
        }
    }
}

/// The walker's overflow clip rectangles and the public list's, in order.
#[test]
fn overflow_clip_rectangles_match() {
    let result = lay_out(NESTED_CLIPS, "");
    assert_eq!(result.page_count(), 1);
    let page = result.pages().next().expect("a page");
    let (walker, api) = clip_rects(&page);
    assert_eq!(walker.len(), 2, "{walker:?}");
    assert_rects_match(&walker, &api, "page 0");
}

/// A padded overflow box sliced across pages. The walker clips to the whole
/// box's padding box, which extends past the page on a cut edge; within the
/// page the clips must agree.
#[test]
fn overflow_clip_rectangles_match_across_pages() {
    let inner: String = (0..25).map(|i| format!("<p>line {i}</p>")).collect();
    let result = lay_out(
        &format!("<div style=\"overflow: hidden; padding: 15px; border: 4px solid\">{inner}</div>"),
        "",
    );
    assert!(result.page_count() > 2, "{} pages", result.page_count());
    for page in result.pages() {
        let page_box = page.geometry().page_box;
        let (w, h) = (f64::from(page_box.width), f64::from(page_box.height));
        let within_page = |r: &[f64; 4]| {
            [
                r[0].clamp(0.0, w),
                r[1].clamp(0.0, h),
                r[2].clamp(0.0, w),
                r[3].clamp(0.0, h),
            ]
        };
        let (walker, api) = clip_rects(&page);
        assert_eq!(walker.len(), 1, "page {}: {walker:?}", page.index());
        let walker: Vec<_> = walker.iter().map(within_page).collect();
        let api: Vec<_> = api.iter().map(within_page).collect();
        assert_rects_match(&walker, &api, &format!("page {}", page.index()));
    }
}

/// A text node laid out as an anonymous flex item draws its own lines.
#[test]
fn flex_item_text() {
    assert_same_order("<div style='display:flex'>text</div>", "");
}

/// The walker clips a multi-column container's columns, and the comparison
/// leaves those clips out with the pops that close them.
#[test]
fn walker_column_clips_are_left_out_of_the_comparison() {
    let result = lay_out(
        "<div style='columns:2; height:60px'><div style='display:grid'>a</div>\
         <p>b</p><p>c</p><p>d</p><p>e</p><p>f</p><p>g</p><p>h</p></div>",
        "",
    );
    let page = result.pages().next().expect("one page");
    let raw = trace(&page);
    assert!(
        raw.iter()
            .any(|e| matches!(e, PaintTraceEvent::PushFragmentainerClip(_))),
        "{raw:?}"
    );
    let steps = walker_steps(&page);
    let pushes = steps.iter().filter(|s| matches!(s, Step::PushClip)).count();
    let pops = steps.iter().filter(|s| matches!(s, Step::PopClip)).count();
    assert_eq!(pushes, pops, "{steps:?}");
}
