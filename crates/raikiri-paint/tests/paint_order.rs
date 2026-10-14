//! `raikiri_html::Page::paint_order` against the order this painter walks
//! the body in.
//!
//! Each fixture is laid out once through `raikiri_html::layout`; for every
//! page the body walk is traced with `trace_paint_order` and both sequences
//! are reduced to a common shape before they are compared.

use raikiri_dom::Document;
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
    trace_paint_order(document, cascade, page_box, origin, Some(page.name())).expect("trace")
}

/// The walker's steps, normalized without column clips and their closing
/// pops so this trace compares the remaining paint traversal.
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
/// Per-cell table background clips wrap repeated decoration events in the
/// public API; the native trace records one box event for the same table part.
fn api_steps(page: &Page<'_>) -> Vec<Step> {
    let (document, _, _, _) = page.paint_inputs();
    let mut steps = Vec::new();
    let mut table_background_clip = false;
    let mut table_background_nodes = std::collections::HashSet::new();
    for event in page.paint_order() {
        let step = match event {
            PaintEvent::PushClip(_, ClipKind::Fragmentainer) => {
                panic!("column clips are not listed yet")
            }
            PaintEvent::PushClip(_, ClipKind::TableCell) => {
                table_background_clip = true;
                continue;
            }
            PaintEvent::PopClip if table_background_clip => {
                table_background_clip = false;
                continue;
            }
            PaintEvent::PushClip(..) => Step::PushClip,
            PaintEvent::PopClip => Step::PopClip,
            PaintEvent::PushOpacity(_) => Step::PushOpacity,
            PaintEvent::PopOpacity => Step::PopOpacity,
            // The walker records no step for a transform.
            PaintEvent::PushTransform(_) | PaintEvent::PopTransform => continue,
            // The walker draws a table part's outer shadows with its box.
            PaintEvent::OuterBoxShadow(_) => continue,
            PaintEvent::Box(fragment) => {
                let node = node_index(fragment.node());
                if table_background_clip && !table_background_nodes.insert(node) {
                    continue;
                }
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

#[test]
fn repeated_table_headers_keep_native_and_public_subtree_order() {
    let rows = "<tr><td>X</td></tr>".repeat(9);
    for header in ["H", "<span style='overflow:hidden;display:block'>H</span>"] {
        let result = assert_same_order(
            &format!(
                "<table style='opacity:0.5'><thead style='background:red'><tr><th>{header}</th></tr></thead><tbody style='background:blue'>{rows}</tbody></table>"
            ),
            "@page{size:100px 100px;margin:0}body{margin:0}table{border-spacing:0}td,th{padding:0;width:20px;vertical-align:top}th{height:10px;font-weight:400}td{height:20px}",
        );
        assert_eq!(result.page_count(), 3);
    }
}

/// A row group's per-cell background clips cover only the cells in the
/// group's slice on the page, and a group with nothing to paint has none.
#[test]
fn table_part_cell_clips_stay_on_their_page() {
    let rows = "<tr><td>X</td></tr>".repeat(9);
    let css = "@page{size:100px 100px;margin:0}body{margin:0}table{border-spacing:2px}td{padding:0;width:20px;height:18px}";
    let clips = |page: &Page<'_>| {
        page.paint_order()
            .into_iter()
            .filter(|event| matches!(event, PaintEvent::PushClip(_, ClipKind::TableCell)))
            .count()
    };
    let result = assert_same_order(
        &format!("<table><tbody style='background:blue'>{rows}</tbody></table>"),
        css,
    );
    assert!(result.page_count() > 1);
    let per_page: Vec<usize> = result.pages().map(|page| clips(&page)).collect();
    // Five rows fit on a page; a row cut by the page edge counts on both.
    assert!(per_page.iter().all(|&count| count <= 6), "{per_page:?}");
    assert!(per_page.iter().sum::<usize>() >= 9, "{per_page:?}");
    let plain = assert_same_order(&format!("<table><tbody>{rows}</tbody></table>"), css);
    assert!(plain.pages().all(|page| clips(&page) == 0));
}

/// The outer shadows of a separated-borders row or row group are listed
/// once, outside the per-cell clips of its background, which would clip
/// them away. The collapsing model ignores them (CSS Backgrounds 3 §7.1).
#[test]
fn separated_table_part_shadows_precede_the_cell_clips() {
    let css = "@page{size:100px 100px;margin:0}body{margin:0}table{border-spacing:2px}td{padding:0;width:20px;height:18px}";
    let shadows = |result: &DocumentLayout| {
        let page = result.page(0).unwrap();
        let events = page.paint_order();
        let first_clip = events
            .iter()
            .position(|event| matches!(event, PaintEvent::PushClip(_, ClipKind::TableCell)));
        let shadows: Vec<_> = events
            .iter()
            .enumerate()
            .filter_map(|(index, event)| match event {
                PaintEvent::OuterBoxShadow(fragment) => Some((index, fragment.node())),
                _ => None,
            })
            .collect();
        if let (Some(&(index, _)), Some(clip)) = (shadows.first(), first_clip) {
            assert!(index < clip, "the shadow precedes the cell clips");
        }
        shadows.len()
    };
    let body = "<table><tr style='box-shadow:4px 4px red;background:blue'><td>X</td><td>Y</td></tr>\
                <tbody style='box-shadow:inset 0 0 0 2px red'><tr><td>Z</td></tr></tbody></table>";
    let separate = assert_same_order(body, css);
    assert_eq!(shadows(&separate), 1);
    let collapse = assert_same_order(body, &format!("{css}table{{border-collapse:collapse}}"));
    assert_eq!(shadows(&collapse), 0);
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

/// CSS 2.1 Appendix E paints every in-flow block background of a layer
/// before the inline content of those blocks, and positioned boxes after
/// both. Text overflowing an empty block stays above the next block's
/// background, and below a later positioned box.
#[test]
fn overflowing_text_paints_above_later_block_backgrounds() {
    let result = assert_same_order(
        "<div id=a style=\"height: 0\">over flow</div>\
         <div id=b style=\"height: 20px; background: yellow\"></div>\
         <div id=c style=\"position: relative; height: 5px; background: lime\"></div>\
         <div id=d style=\"background: gray\">after</div>\
         <div id=e style=\"overflow: hidden; background: teal\"><p id=f>clipped</p></div>",
        "",
    );
    let page = result.pages().next().expect("a page");
    let steps = walker_steps(&page);
    let at = |step: Step| {
        steps
            .iter()
            .position(|s| *s == step)
            .unwrap_or_else(|| panic!("{step:?} in {}", describe(&page, &steps)))
    };
    let [a, b, c, d, e, f] = ["a", "b", "c", "d", "e", "f"].map(|id| by_id(&result, id));
    assert!(at(Step::Box(b)) < at(Step::Text(a)));
    assert!(at(Step::Text(a)) < at(Step::Box(c)));
    assert!(at(Step::Box(d)) < at(Step::Text(d)));
    // A clipping block closes its own layer, so its content stays inside
    // the clip, before the blocks that follow it.
    assert!(at(Step::Box(e)) < at(Step::Text(f)));
}

/// Floats and in-flow blocks that clip paint below the inline content of
/// their layer (CSS 2.1 Appendix E steps 4, 5 and 7), and blocks inside an
/// inline-block or a table cell defer their content like any other block.
#[test]
fn floats_clipping_blocks_and_atomic_containers_keep_text_above() {
    let result = assert_same_order(
        "<div id=a style=\"height: 0\">over flow</div>\
         <div id=b style=\"float: left; width: 50px; height: 20px; background: yellow\"></div>\
         <div id=c style=\"overflow: hidden; height: 20px; background: teal\"></div>",
        "",
    );
    let page = result.pages().next().expect("a page");
    let steps = walker_steps(&page);
    let at = |step: Step| {
        steps
            .iter()
            .position(|s| *s == step)
            .unwrap_or_else(|| panic!("{step:?} in {}", describe(&page, &steps)))
    };
    let [a, b, c] = ["a", "b", "c"].map(|id| by_id(&result, id));
    assert!(at(Step::Box(b)) < at(Step::Text(a)));
    assert!(at(Step::Box(c)) < at(Step::Text(a)));

    let result = assert_same_order(
        "<div style=\"display: inline-block; width: 80px\">\
         <div id=e style=\"height: 0\">inside</div>\
         <div id=f style=\"height: 20px; background: gray\"></div></div>\
         <table><tr><td><div id=h style=\"height: 0\">cell</div>\
         <div id=i style=\"height: 20px; background: lime\"></div></td></tr></table>",
        "",
    );
    let page = result.pages().next().expect("a page");
    let steps = walker_steps(&page);
    let at = |step: Step| {
        steps
            .iter()
            .position(|s| *s == step)
            .unwrap_or_else(|| panic!("{step:?} in {}", describe(&page, &steps)))
    };
    let [e, f, h, i] = ["e", "f", "h", "i"].map(|id| by_id(&result, id));
    assert!(at(Step::Box(f)) < at(Step::Text(e)));
    assert!(at(Step::Box(i)) < at(Step::Text(h)));
}

/// A `display: contents` element has no box, so the blocks inside it defer
/// their content and stay in the layer of its parent.
#[test]
fn display_contents_stays_inside_the_deferral_layer() {
    let result = assert_same_order(
        "<div id=a style=\"height: 0\">over flow</div>\
         <section style=\"display: contents\">\
         <div id=b style=\"height: 0\">inner</div>\
         <div id=c style=\"height: 20px; background: teal\"></div></section>",
        "",
    );
    let page = result.pages().next().expect("a page");
    let steps = walker_steps(&page);
    let at = |step: Step| {
        steps
            .iter()
            .position(|s| *s == step)
            .unwrap_or_else(|| panic!("{step:?} in {}", describe(&page, &steps)))
    };
    let [a, b, c] = ["a", "b", "c"].map(|id| by_id(&result, id));
    assert!(at(Step::Box(c)) < at(Step::Text(a)));
    assert!(at(Step::Box(c)) < at(Step::Text(b)));
}

/// Compares both sequences of a fixture without extra style.
fn same_order(body: &str) -> DocumentLayout {
    assert_same_order(body, "")
}

/// The position of `step` in the walker's sequence of the first page.
fn step_at(result: &DocumentLayout, step: Step) -> usize {
    let page = result.pages().next().expect("a page");
    let steps = walker_steps(&page);
    steps
        .iter()
        .position(|s| *s == step)
        .unwrap_or_else(|| panic!("{step:?} in {}", describe(&page, &steps)))
}

/// A block that clips hands its text to the enclosing layer inside its
/// clip, so a later block's background stays below the clipped text.
#[test]
fn clipped_text_paints_above_later_block_backgrounds() {
    let result = same_order(
        "<div id=a style=\"overflow: hidden; height: 20px\">\
         <div id=b style=\"height: 0\">clip text</div></div>\
         <div id=c style=\"margin-top: -10px; height: 20px; background: teal\"></div>",
    );
    let [a, b, c] = ["a", "b", "c"].map(|id| by_id(&result, id));
    assert!(step_at(&result, Step::Box(a)) < step_at(&result, Step::Box(c)));
    assert!(step_at(&result, Step::Box(c)) < step_at(&result, Step::Text(b)));
    let page = result.pages().next().expect("a page");
    let steps = walker_steps(&page);
    let text = step_at(&result, Step::Text(b));
    assert_eq!(steps[text - 1], Step::PushClip);
    assert_eq!(steps[text + 1], Step::PopClip);
}

/// A positioned box inside a clipping block waits for the end of the
/// enclosing layer, still inside the block's clip.
#[test]
fn positioned_boxes_in_clipping_blocks_keep_the_clip() {
    let result = same_order(
        "<div id=a style=\"overflow: hidden; height: 20px\">\
         <div id=b style=\"position: relative; height: 5px; background: red\"></div></div>\
         <div id=c style=\"height: 20px; background: teal\"></div>",
    );
    let [b, c] = ["b", "c"].map(|id| by_id(&result, id));
    assert!(step_at(&result, Step::Box(c)) < step_at(&result, Step::Box(b)));
    let page = result.pages().next().expect("a page");
    let steps = walker_steps(&page);
    assert_eq!(steps[step_at(&result, Step::Box(b)) - 1], Step::PushClip);
}

/// A float paints its own content, but the positioned boxes inside it
/// belong to the enclosing layer and paint after its text.
#[test]
fn positioned_boxes_in_floats_paint_after_the_layer_text() {
    let result = same_order(
        "<div id=a style=\"height: 0\">over</div>\
         <div id=f style=\"float: left; width: 50px; height: 20px; background: gray\">\
         <div id=p style=\"position: relative; height: 5px; background: red\"></div></div>",
    );
    let [a, f, p] = ["a", "f", "p"].map(|id| by_id(&result, id));
    assert!(step_at(&result, Step::Box(f)) < step_at(&result, Step::Text(a)));
    assert!(step_at(&result, Step::Text(a)) < step_at(&result, Step::Box(p)));
}

/// The backgrounds of a table and its cells are block backgrounds of the
/// layer: text before the table and in earlier cells stays above them.
#[test]
fn table_backgrounds_paint_below_the_layer_text() {
    let result = same_order(
        "<div id=a style=\"height: 0\">over</div>\
         <table id=t style=\"border-spacing: 0; background: lime\"><tr>\
         <td id=c1 style=\"width: 20px\">overflowing_text</td>\
         <td id=c2 style=\"width: 20px; background: teal\"></td></tr></table>",
    );
    let [a, t, c1, c2] = ["a", "t", "c1", "c2"].map(|id| by_id(&result, id));
    assert!(step_at(&result, Step::Box(t)) < step_at(&result, Step::Text(a)));
    assert!(step_at(&result, Step::Box(c2)) < step_at(&result, Step::Text(c1)));
    assert!(step_at(&result, Step::Text(a)) < step_at(&result, Step::Text(c1)));

    // A cell that clips paints its background below the earlier cells' text
    // too.
    let result = same_order(
        "<table style=\"border-spacing: 0\"><tr>\
         <td id=c1 style=\"width: 20px\">overflowing_text</td>\
         <td id=c2 style=\"width: 20px; overflow: hidden; background: teal\">x</td></tr></table>",
    );
    let [c1, c2] = ["c1", "c2"].map(|id| by_id(&result, id));
    assert!(step_at(&result, Step::Box(c2)) < step_at(&result, Step::Text(c1)));
    assert!(step_at(&result, Step::Text(c1)) < step_at(&result, Step::Text(c2)));

    // An inline table keeps its cells' text above its cells' backgrounds
    // too, inside its own layer.
    let result = same_order(
        "<div>a <span style=\"display: inline-table; border-spacing: 0\">\
         <span style=\"display: table-row\">\
         <span id=c1 style=\"display: table-cell; width: 20px\">overflowing_text</span>\
         <span id=c2 style=\"display: table-cell; width: 20px; background: teal\"></span>\
         </span></span></div>",
    );
    let [c1, c2] = ["c1", "c2"].map(|id| by_id(&result, id));
    assert!(step_at(&result, Step::Box(c2)) < step_at(&result, Step::Text(c1)));
}

/// A block inside a paragraph's inline content paints its background before
/// the paragraph's lines, whether the paragraph defers its content or not.
#[test]
fn blocks_inside_paragraphs_paint_before_the_lines() {
    for style in ["height: 0", "position: relative; height: 0"] {
        let result = same_order(&format!(
            "<div id=m style=\"{style}\">over\
             <div id=k style=\"background: gray; height: 10px\">inner</div>tail</div>"
        ));
        let [m, k] = ["m", "k"].map(|id| by_id(&result, id));
        assert!(step_at(&result, Step::Box(k)) < step_at(&result, Step::Text(m)));
        assert!(step_at(&result, Step::Text(m)) < step_at(&result, Step::Text(k)));
    }
}

/// A paragraph with content positioned below its blocks keeps the blocks
/// with that content.
#[test]
fn blocks_inside_paragraphs_with_negative_content_stay_with_it() {
    let result = same_order(
        "<div id=m>over<span style=\"position: relative; z-index: -1\">x</span>\
         <div id=k style=\"background: gray; height: 10px\"></div></div>",
    );
    let [m, k] = ["m", "k"].map(|id| by_id(&result, id));
    assert!(step_at(&result, Step::Text(m)) < step_at(&result, Step::Box(k)));
}

/// A box in a paragraph's lines that closes the deferred text of the
/// paragraph's blocks paints above that text.
#[test]
fn closing_boxes_in_paragraph_lines_paint_above_block_text() {
    let result = same_order(
        "<div id=m style=\"position: relative\">over<div id=k style=\"height: 0\">inner</div>\
         <span id=o style=\"display: inline-block; opacity: 0.5; background: blue\">x</span></div>",
    );
    let [k, o] = ["k", "o"].map(|id| by_id(&result, id));
    assert!(step_at(&result, Step::Text(k)) < step_at(&result, Step::Box(o)));
}

/// A positioned box waits for the end of its layer, so a block that
/// follows its parent paints its background below it.
#[test]
fn positioned_boxes_paint_after_later_block_backgrounds() {
    let result = same_order(
        "<div><div id=a style=\"height: 0\">over</div>\
         <div id=r style=\"position: relative; height: 5px; background: red\"></div>\
         <div id=o style=\"opacity: 0.5; height: 5px; background: blue\"></div></div>\
         <div id=d style=\"height: 20px; background: gray\"></div>",
    );
    let [a, r, o, d] = ["a", "r", "o", "d"].map(|id| by_id(&result, id));
    assert!(step_at(&result, Step::Box(d)) < step_at(&result, Step::Text(a)));
    assert!(step_at(&result, Step::Text(a)) < step_at(&result, Step::Box(r)));
    assert!(step_at(&result, Step::Text(a)) < step_at(&result, Step::Box(o)));
}

/// Positioned boxes inside a flex container or a flex item keep tree order
/// with the positioned boxes before them.
#[test]
fn positioned_boxes_in_flex_containers_keep_tree_order() {
    let green = "<div id=g style=\"position: absolute; width: 10px; height: 10px; \
                 background: green\"></div>";
    for inner in [green.to_string(), format!("<div>{green}</div>")] {
        let result = same_order(&format!(
            "<div style=\"position: relative; height: 20px\">\
             <div id=r style=\"position: absolute; width: 20px; height: 20px; background: red\"></div>\
             <div style=\"display: flex; height: 20px\">{inner}</div></div>\
             <div id=d style=\"height: 20px; background: gray\"></div>"
        ));
        let [r, g, d] = ["r", "g", "d"].map(|id| by_id(&result, id));
        assert!(step_at(&result, Step::Box(d)) < step_at(&result, Step::Box(g)));
        assert!(step_at(&result, Step::Box(r)) < step_at(&result, Step::Box(g)));
    }
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

#[test]
fn relative_box_paints_at_its_public_fragment_coordinates() {
    for (insets, reference_insets, x, y) in [
        ("left:-10px;top:-10px", "left:0;top:0", 15.0, 25.0),
        ("left:10px;top:12px", "left:20px;top:22px", 35.0, 47.0),
        ("right:10px;bottom:10px", "left:0;top:0", 15.0, 25.0),
    ] {
        let body = "<div class='parent'><div id='child'>X<div class='nested'></div></div></div>";
        let css = "@page { margin:0 } body { margin:0 } .parent { position:absolute;left:10px;top:20px;width:100px;height:80px;padding:10px;border:5px solid red } #child { width:20px;height:20px;background:green;border:2px solid black } .nested { width:10px;height:10px;background:blue }";
        let result = lay_out(
            body,
            &format!("{css} #child {{ position:relative;{insets} }}"),
        );
        let child = by_id(&result, "child");
        let page = result.pages().next().expect("a page");
        let rect = page
            .fragments()
            .find(|fragment| {
                fragment.kind() == FragmentKind::Box && node_index(fragment.node()) == child
            })
            .expect("child fragment")
            .paint_rect();
        assert_eq!(
            (rect.x, rect.y, rect.width, rect.height),
            (x, y, 24.0, 24.0)
        );

        let pixels = |page: Page<'_>| {
            let (document, cascade, page_box, origin) = page.paint_inputs();
            anyrender::render_to_buffer::<anyrender_vello_cpu::VelloCpuImageRenderer, _>(
                |scene| {
                    raikiri_paint::paint_single_page_with_origin(
                        scene, document, cascade, page_box, origin,
                    )
                    .expect("paint");
                },
                300,
                200,
            )
        };
        let reference = lay_out(
            body,
            &format!("{css} #child {{ position:absolute;{reference_insets} }}"),
        );
        assert_eq!(
            pixels(page),
            pixels(reference.pages().next().expect("reference page")),
            "background, border, text and descendants for {insets}"
        );
    }
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
fn overflow_clip_keeps_padding_and_excludes_border_in_both_paths() {
    let result = lay_out(
        "<div style='position:absolute;left:10px;top:20px;width:100px;height:50px;padding:10px;border:5px solid red;overflow:hidden'><div style='width:10px;height:10px;background:green'></div></div>",
        "@page { margin: 0 } body { margin: 0 }",
    );
    let page = result.pages().next().expect("a page");
    let (walker, api) = clip_rects(&page);
    let expected = [[15.0, 25.0, 135.0, 95.0]];
    assert_eq!(walker, expected);
    assert_eq!(api, expected);
}

#[test]
fn overflow_clip_rectangles_match() {
    let result = lay_out(NESTED_CLIPS, "");
    assert_eq!(result.page_count(), 1);
    let page = result.pages().next().expect("a page");
    let (walker, api) = clip_rects(&page);
    // The middle box's clip opens again around the translucent box, which
    // waits for the end of the outer group's layer.
    assert_eq!(walker.len(), 3, "{walker:?}");
    assert_rects_match(&walker, &api, "page 0");
}

/// Every page's overflow clip rectangles, compared exactly: the walker
/// clips to the whole box's padding box, which runs past the page where a
/// page break cuts the box.
fn assert_clip_rects_on_every_page(body: &str) {
    let result = lay_out(body, "");
    assert!(result.page_count() > 1, "{} pages", result.page_count());
    for page in result.pages() {
        let (walker, api) = clip_rects(&page);
        // The clip opens around the box's subtree, and again around the
        // text it defers to the end of the enclosing layer when the page
        // has any.
        let has_text = walker_steps(&page)
            .iter()
            .any(|step| matches!(step, Step::Text(_)));
        assert_eq!(
            walker.len(),
            1 + usize::from(has_text),
            "page {}: {walker:?}",
            page.index()
        );
        assert_rects_match(&walker, &api, &format!("page {}", page.index()));
    }
}

/// A padded overflow box that starts at the top of the first page and runs
/// over several pages.
#[test]
fn overflow_clip_rectangles_match_across_pages() {
    let inner: String = (0..25).map(|i| format!("<p>line {i}</p>")).collect();
    assert_clip_rects_on_every_page(&format!(
        "<div style=\"overflow: hidden; padding: 15px; border: 4px solid\">{inner}</div>"
    ));
}

/// A padded overflow box that starts close to the bottom of a page: the part
/// before the break is shorter than the page margin plus the padding.
#[test]
fn overflow_clip_rectangles_match_for_a_box_starting_near_a_page_bottom() {
    let inner: String = (0..5).map(|i| format!("<p>line {i}</p>")).collect();
    for spacer in [130, 150] {
        assert_clip_rects_on_every_page(&format!(
            "<div style=\"height: {spacer}px\"></div>\
             <div style=\"overflow: hidden; padding: 15px; border: 4px solid\">{inner}</div>"
        ));
    }
}

/// A box whose bottom padding is cut by a page break.
#[test]
fn overflow_clip_rectangles_match_for_padding_cut_by_a_break() {
    assert_clip_rects_on_every_page(
        "<div style=\"height: 150px\"></div>\
         <div style=\"overflow: hidden; padding-bottom: 50px; height: 10px\"><p>a</p></div>",
    );
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

/// Boxes beside the lines of a paragraph with inline opacity groups are
/// traced where the painter places them: a box with a negative stack level
/// below the text, the others inside the groups around them. (The walker
/// traces a paragraph's text once, so the group holds no text here.)
#[test]
fn atomic_inlines_beside_inline_opacity_groups() {
    assert_same_order(
        "<p>A<i style=\"display:inline-block;width:5px;height:5px;position:relative;\
         z-index:-1;background:red\"></i>B<span style=\"opacity:.5\">\
         <b style=\"display:inline-block;width:5px;height:5px;background:blue\"></b></span></p>",
        "",
    );
}
