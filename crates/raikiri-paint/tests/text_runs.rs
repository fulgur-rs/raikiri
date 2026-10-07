//! `raikiri_html::Page::text_runs` against what this painter draws.
//!
//! Each fixture is laid out once through `raikiri_html::layout`; every page is
//! then painted into a recording scene from the same layout, and the glyph
//! positions the runs describe are compared with the recorded glyph draws.

use anyrender::Scene;
use anyrender::recording::RenderCommand;
use kurbo::Point;
use raikiri_dom::{CounterSnapshotBudget, PositionedLines};
use raikiri_html::{
    DocumentLayout, FontCollectionBuilder, FragmentKind, GeneratedKind, LayoutConfig,
    LayoutOptions, LayoutStatus, NodeId, NodeKind, Page, PageDefaults, PositionedGlyphRun,
    RenderFonts, RenderResources, RunSource, WarningKind, layout, parse_html_with_resources,
};
use raikiri_paint::paint_single_page_with_origin_and_page;
use std::collections::{HashMap, HashSet};

const AHEM: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../raikiri-dom/tests/data/text-autospace/Ahem.ttf"
));
const NOTO: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../raikiri-html/tests/data/NotoSansTest-Regular.ttf"
));

const TOLERANCE: f64 = 1e-3;

fn fonts() -> RenderFonts {
    FontCollectionBuilder::new()
        .font_bytes("Ahem", AHEM.to_vec())
        .font_bytes("Noto Sans Test", NOTO.to_vec())
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

/// `(glyph id, x, y)` of every glyph the painter draws for `page`.
fn drawn(page: &Page<'_>, page_count: u32) -> Vec<(u32, f64, f64)> {
    let (document, cascade, page_box, origin) = page.paint_inputs();
    let mut scene = Scene::new();
    let mut budget = CounterSnapshotBudget::default();
    paint_single_page_with_origin_and_page(
        &mut scene,
        document,
        cascade,
        page_box,
        origin,
        page.index(),
        page_count,
        page.index() % 2 == 1,
        &mut budget,
    )
    .expect("paint");
    let mut out = Vec::new();
    for command in &scene.commands {
        if let RenderCommand::GlyphRun(run) = command {
            for glyph in &run.glyphs {
                let point = run.transform * Point::new(f64::from(glyph.x), f64::from(glyph.y));
                out.push((glyph.id, point.x, point.y));
            }
        }
    }
    out
}

/// `(glyph id, x, y)` of the glyphs of `run`, from its origin, advances and
/// offsets.
fn expanded(run: &PositionedGlyphRun<'_>) -> Vec<(u32, f64, f64)> {
    let mut pen = f64::from(run.origin.0);
    run.glyphs
        .iter()
        .map(|glyph| {
            let placed = (
                glyph.id,
                pen + f64::from(glyph.x_offset),
                f64::from(run.origin.1) + f64::from(glyph.y_offset),
            );
            pen += f64::from(glyph.advance);
            placed
        })
        .collect()
}

fn close(a: &(u32, f64, f64), b: &(u32, f64, f64)) -> bool {
    a.0 == b.0 && (a.1 - b.1).abs() < TOLERANCE && (a.2 - b.2).abs() < TOLERANCE
}

/// Every glyph of every run is drawn on its page where the run puts it, and
/// every glyph drawn inside the page's content box is in a run, unless it is
/// on a line that straddles the page boundary and belongs to another page.
fn assert_runs_match_paint(result: &DocumentLayout) -> usize {
    let mut total = 0;
    // Each page's runs, moved into the shared flow space.
    let flow: Vec<(u32, f64, f64)> = result
        .pages()
        .flat_map(|page| {
            let (_, _, _, origin) = page.paint_inputs();
            let shift = f64::from(origin) - f64::from(page.geometry().content_box.y);
            page.text_runs()
                .iter()
                .flat_map(expanded)
                .map(move |(id, x, y)| (id, x, y + shift))
                .collect::<Vec<_>>()
        })
        .collect();
    for page in result.pages() {
        let runs = page.text_runs();
        let expected: Vec<_> = runs.iter().flat_map(expanded).collect();
        let mut drawn = drawn(&page, result.page_count());
        for glyph in &expected {
            let found = drawn.iter().position(|other| close(glyph, other));
            let Some(found) = found else {
                panic!(
                    "page {}: glyph {glyph:?} of the runs is not drawn; drawn: {drawn:?}",
                    page.index()
                );
            };
            drawn.swap_remove(found);
        }
        // Glyphs of other pages' lines are drawn too, clipped away; only the
        // ones inside this page's content box must be accounted for.
        let content = page.geometry().content_box;
        let stray: Vec<_> = drawn
            .iter()
            .filter(|(_, _, y)| {
                *y > f64::from(content.y) && *y <= f64::from(content.y + content.height)
            })
            .filter(|glyph| {
                let (_, _, _, origin) = page.paint_inputs();
                let shift = f64::from(origin) - f64::from(content.y);
                let in_flow = (glyph.0, glyph.1, glyph.2 + shift);
                !flow.iter().any(|other| close(&in_flow, other))
            })
            .collect();
        assert!(
            stray.is_empty(),
            "page {}: drawn glyphs missing from the runs: {stray:?}",
            page.index()
        );
        total += expected.len();
    }
    total
}

/// Identifies a run by the slice of processed text it covers.
fn key(text: &str) -> (usize, usize) {
    (text.as_ptr() as usize, text.len())
}

/// For every run of every paragraph that is not repeated: its key, its
/// paragraph root, and its line index.
fn all_runs(result: &DocumentLayout) -> HashMap<(usize, usize), (usize, usize)> {
    let page = result.pages().next().expect("a page");
    let (document, cascade, _, _) = page.paint_inputs();
    let mut out = HashMap::new();
    for root in 0..document.node_count() {
        if !document
            .get_node(root)
            .is_some_and(|node| node.is_ifc_root())
        {
            continue;
        }
        let Some(lines) = PositionedLines::new(document, cascade, root, None) else {
            continue;
        };
        for line in lines.lines() {
            for run in &line.runs {
                let text = &line.line.text()[run.run.text_range()];
                out.insert(key(text), (root, line.index));
            }
        }
    }
    out
}

/// The union of the pages' runs is every run of the document once, and each
/// text run lies on a line its page's text fragment covers.
fn assert_runs_partition_the_document(result: &DocumentLayout) {
    let all = all_runs(result);
    let mut seen = HashSet::new();
    for page in result.pages() {
        let (document, _, _, _) = page.paint_inputs();
        let fragments: HashMap<NodeId, std::ops::Range<u32>> = page
            .fragments()
            .filter(|fragment| fragment.kind() == FragmentKind::Text)
            .filter_map(|fragment| Some((fragment.node(), fragment.line_range()?)))
            .collect();
        for run in page.text_runs() {
            let run_key = key(run.text);
            assert!(
                seen.insert(run_key),
                "page {}: run {:?} appears twice",
                page.index(),
                run.text
            );
            let &(root, line) = all.get(&run_key).expect("a run of the document");
            let RunSource::Text(node) = run.source else {
                continue;
            };
            // A text node of only white space has no fragment.
            if page
                .dom()
                .text(node)
                .is_some_and(|text| text.trim().is_empty())
            {
                continue;
            }
            let lines = document
                .ifc_text_lines(node.0 as usize)
                .expect("text node lines");
            assert_eq!(lines.root, root);
            let local = lines
                .lines
                .iter()
                .position(|owned| owned.line == line)
                .expect("the run's line is one of its text node's lines")
                as u32;
            let range = fragments.get(&node).expect("a text fragment on the page");
            assert!(
                range.contains(&local),
                "page {}: line {local} of {node:?} is outside {range:?}",
                page.index()
            );
        }
    }
    assert_eq!(seen.len(), all.len(), "every run appears on some page");
}

/// Byte ranges of the glyphs start at 0, never go backwards in logical order,
/// and end at the end of the run's text.
fn assert_text_ranges_cover_the_text(result: &DocumentLayout) {
    for page in result.pages() {
        for run in page.text_runs() {
            let mut ranges: Vec<_> = run.glyphs.iter().map(|g| g.text_range.clone()).collect();
            ranges.sort_by_key(|range| (range.start, range.end));
            let first = ranges.first().expect("a glyph");
            assert_eq!(first.start, 0, "{:?}", run.text);
            assert_eq!(ranges.last().expect("a glyph").end, run.text.len());
            for pair in ranges.windows(2) {
                assert!(pair[0].start == pair[1].start || pair[0].end == pair[1].start);
            }
            for range in &ranges {
                assert!(run.text.get(range.clone()).is_some());
            }
        }
    }
}

fn check(result: &DocumentLayout) -> usize {
    let glyphs = assert_runs_match_paint(result);
    assert_runs_partition_the_document(result);
    assert_text_ranges_cover_the_text(result);
    glyphs
}

fn paragraphs(count: usize, words: usize) -> String {
    (0..count)
        .map(|i| format!("<p>{}</p>", vec![format!("w{i}x"); words].join(" ")))
        .collect()
}

#[test]
fn single_page_paragraphs_match_paint() {
    let result = lay_out(&paragraphs(3, 4), "");
    assert_eq!(result.page_count(), 1);
    assert!(check(&result) > 0);
}

#[test]
fn overflowing_paragraphs_match_paint_on_every_page() {
    let result = lay_out(&paragraphs(12, 30), "");
    assert!(result.page_count() >= 3, "{} pages", result.page_count());
    check(&result);
    for page in result.pages() {
        assert!(!page.text_runs().is_empty(), "page {}", page.index());
    }
}

#[test]
fn forced_page_breaks_match_paint() {
    let result = lay_out(
        "<p>first page</p><p style=\"break-before:page\">second page</p>\
         <p style=\"break-before:page\">third page</p>",
        "",
    );
    assert_eq!(result.page_count(), 3);
    check(&result);
    let texts: Vec<Vec<&str>> = result
        .pages()
        .map(|page| page.text_runs().iter().map(|run| run.text).collect())
        .collect();
    assert_eq!(texts[1].concat().trim(), "second page");
}

#[test]
fn mixed_inline_styles_match_paint_and_carry_their_colors() {
    let result = lay_out(
        "<p>plain <b>bold</b> <span style=\"color: rgb(0, 128, 0)\">green</span> \
         <i>italic</i></p>",
        "",
    );
    check(&result);
    let page = result.pages().next().expect("page");
    let runs = page.text_runs();
    for run in &runs {
        let RunSource::Text(node) = run.source else {
            panic!("only text runs expected");
        };
        assert_eq!(run.color, page.computed(node).expect("computed").color);
    }
    let green = runs
        .iter()
        .find(|run| run.text == "green")
        .expect("the green run");
    assert_eq!((green.color.r, green.color.g, green.color.b), (0, 128, 0));
    let bold = runs
        .iter()
        .find(|run| run.text == "bold")
        .expect("the bold run");
    // Ahem has no bold face; the painter emboldens it.
    assert!(bold.synthesis.embolden);
    let ids: HashSet<_> = runs.iter().map(|run| run.font.id).collect();
    assert_eq!(ids.len(), 1, "every run is in the Ahem face");
    assert_eq!(runs[0].font.data.as_bytes(), AHEM);
    assert_eq!((*runs[0].font.data.to_arc()).as_ref(), AHEM);
}

#[test]
fn generated_content_runs_match_paint_with_the_pseudo_element_color() {
    let result = lay_out(
        "<p id=\"p\">body text</p>",
        "#p::before { content: \"before \"; color: rgb(0, 0, 255) } \
         #p::after { content: \" after\"; color: rgb(255, 0, 0) }",
    );
    check(&result);
    let page = result.pages().next().expect("page");
    let runs = page.text_runs();
    let generated = runs
        .iter()
        .find(|run| matches!(run.source, RunSource::Generated(_, GeneratedKind::Before)))
        .expect("a ::before run");
    assert_eq!(generated.text.trim(), "before");
    let RunSource::Generated(element, _) = generated.source else {
        unreachable!();
    };
    assert_eq!(page.dom().attr(element, "id"), Some("p"));
    assert_eq!(
        (generated.color.r, generated.color.g, generated.color.b),
        (0, 0, 255)
    );
    let after = runs
        .iter()
        .find(|run| matches!(run.source, RunSource::Generated(_, GeneratedKind::After)))
        .expect("an ::after run");
    assert_eq!(after.text.trim(), "after");
    assert_eq!((after.color.r, after.color.g, after.color.b), (255, 0, 0));
    let text = runs
        .iter()
        .find(|run| matches!(run.source, RunSource::Text(_)))
        .expect("a text run");
    assert_eq!((text.color.r, text.color.g, text.color.b), (0, 0, 0));
}

#[test]
fn body_margin_runs_match_paint() {
    let result = lay_out(
        &format!("{}<div><p>nested paragraph</p></div>", paragraphs(8, 20)),
        "body { margin: 20px }",
    );
    assert!(result.page_count() >= 2);
    check(&result);
}

#[test]
fn body_text_with_a_margin_matches_paint() {
    let result = lay_out("text directly in the body", "body { margin: 20px }");
    assert!(check(&result) > 0);
    let page = result.pages().next().expect("page");
    let content = page.geometry().content_box;
    assert_eq!(page.text_runs()[0].origin.0, content.x + 20.0);
}

/// The box fragment of the element with id `id` on the first page.
fn box_rect(result: &DocumentLayout, id: &str) -> raikiri_html::PaintRect {
    let page = result.pages().next().expect("page");
    let node = by_id(&page, id);
    let fragment = page
        .fragments()
        .find(|fragment| fragment.node() == node && fragment.kind() == FragmentKind::Box)
        .unwrap_or_else(|| panic!("no box fragment for {id}"));
    assert_eq!(fragment.paint_rect(), fragment.rect());
    fragment.rect()
}

#[test]
fn ua_body_margin_places_block_content_like_the_painter() {
    let result = lay_out(
        "<p id='p'>para</p><div id='full' style='width: 100%; height: 5px'></div>",
        "p { margin: 0 }",
    );
    assert!(check(&result) > 0);
    let page = result.pages().next().expect("page");
    let content = page.geometry().content_box;
    let p = box_rect(&result, "p");
    assert_eq!((p.x, p.width), (content.x + 8.0, content.width - 16.0));
    let full = box_rect(&result, "full");
    assert_eq!(
        (full.x, full.width),
        (content.x + 8.0, content.width - 16.0)
    );
    assert_eq!(page.text_runs()[0].origin.0, content.x + 8.0);
}

#[test]
fn ua_body_margin_places_direct_body_text_with_or_without_a_background() {
    for css in ["", "body { background-color: rgb(255, 255, 0) }"] {
        let result = lay_out("text directly in the body", css);
        assert!(check(&result) > 0);
        let page = result.pages().next().expect("page");
        let content = page.geometry().content_box;
        assert_eq!(page.text_runs()[0].origin.0, content.x + 8.0, "{css:?}");
    }
}

#[test]
fn zero_and_percentage_body_margins_place_content_like_the_painter() {
    for (css, margin) in [("body { margin: 0 }", 0.0), ("body { margin: 0 10% }", 0.1)] {
        let result = lay_out("<p id='p'>para</p>", css);
        assert!(check(&result) > 0);
        let page = result.pages().next().expect("page");
        let content = page.geometry().content_box;
        let inset = content.width * margin;
        let p = box_rect(&result, "p");
        assert!((p.x - (content.x + inset)).abs() < 1e-3, "{css}: {p:?}");
        assert!(
            (p.width - (content.width - 2.0 * inset)).abs() < 1e-3,
            "{css}: {p:?}"
        );
        assert!((page.text_runs()[0].origin.0 - p.x).abs() < 1e-3, "{css}");
    }
}

#[test]
fn right_to_left_body_margin_keeps_content_off_the_right_edge() {
    let result = lay_out(
        "<div id='box' style='width: 50px; height: 5px'></div><p>\u{5d0}\u{5d1}</p>",
        "body { direction: rtl; margin: 0 20px 0 4px }",
    );
    assert!(check(&result) > 0);
    let page = result.pages().next().expect("page");
    let content = page.geometry().content_box;
    let right = content.x + content.width - 20.0;
    let block = box_rect(&result, "box");
    assert_eq!(block.x + block.width, right);
    let run = &page.text_runs()[0];
    assert!((run.origin.0 + run.advance - right).abs() < 1e-3, "{run:?}");
}

#[test]
fn right_to_left_runs_match_paint_left_to_right() {
    let result = lay_out(
        "<p dir=\"rtl\">\u{5d0}\u{5d1}\u{5d2} \u{5d3}\u{5d4} abc</p>",
        "",
    );
    check(&result);
    let page = result.pages().next().expect("page");
    for run in page.text_runs() {
        // Ahem gives every glyph a 1em advance, so glyphs listed left to
        // right sit exactly at their accumulated advances.
        for glyph in &run.glyphs {
            assert!(glyph.x_offset.abs() < 1e-3, "{run:?}");
        }
    }
}

#[test]
fn proportional_font_runs_match_paint() {
    let result = lay_out(
        &paragraphs(6, 25),
        "p { font-family: 'Noto Sans Test'; font-size: 13px; line-height: 17px }",
    );
    check(&result);
}

#[test]
fn repeated_paragraphs_appear_on_every_page() {
    let result = lay_out(
        &format!(
            "<div style=\"position: fixed; top: 5px; left: 7px\">header</div>{}",
            paragraphs(10, 30)
        ),
        "",
    );
    assert!(result.page_count() >= 2);
    for page in result.pages() {
        let headers = page
            .text_runs()
            .into_iter()
            .filter(|run| run.text == "header")
            .count();
        assert_eq!(headers, 1, "page {}", page.index());
    }
    assert_runs_match_paint(&result);
}

/// The text node ids under the element with `id`.
fn text_nodes(page: &Page<'_>, id: &str) -> HashSet<NodeId> {
    fn find(dom: &raikiri_html::DomView<'_>, node: NodeId, id: &str) -> Option<NodeId> {
        if dom.attr(node, "id") == Some(id) {
            return Some(node);
        }
        dom.children(node).find_map(|child| find(dom, child, id))
    }
    fn collect(dom: &raikiri_html::DomView<'_>, node: NodeId, out: &mut HashSet<NodeId>) {
        if dom.kind(node) == Some(NodeKind::Text) {
            out.insert(node);
        }
        for child in dom.children(node) {
            collect(dom, child, out);
        }
    }
    let dom = page.dom();
    let root = find(&dom, dom.root(), id).expect("element");
    let mut out = HashSet::new();
    collect(&dom, root, &mut out);
    out
}

fn assert_omitted(body: &str, css: &str, id: &str) {
    let result = lay_out(body, css);
    let page = result.pages().next().expect("page");
    let omitted = text_nodes(&page, id);
    assert!(!omitted.is_empty());
    for page in result.pages() {
        for run in page.text_runs() {
            if let RunSource::Text(node) = run.source {
                assert!(!omitted.contains(&node), "{:?} was reported", run.text);
            }
        }
    }
    assert!(
        page.text_runs().iter().any(|run| run.text.contains("kept")),
        "the other paragraph keeps its runs"
    );
    let warnings: Vec<_> = result
        .warnings()
        .iter()
        .filter(|warning| matches!(warning.kind, WarningKind::TextRunsOmitted))
        .collect();
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    let root = warnings[0].node_id.expect("the paragraph root");
    let dom = page.dom();
    assert!(
        dom.attr(root, "id") == Some(id) || omitted.iter().any(|&n| dom.parent(n) == Some(root))
    );
}

#[test]
fn vertical_paragraphs_have_no_runs_and_a_warning() {
    assert_omitted(
        "<p>kept</p><p id=\"v\" style=\"writing-mode: vertical-rl; height: 100px\">vertical</p>",
        "",
        "v",
    );
}

#[test]
fn multicol_paragraphs_have_no_runs_and_a_warning() {
    assert_omitted(
        &format!(
            "<p>kept</p><div id=\"m\" style=\"columns: 2; height: 40px\">{}</div>",
            vec!["word"; 40].join(" ")
        ),
        "",
        "m",
    );
}

#[test]
fn relatively_positioned_inline_elements_have_no_runs_and_a_warning() {
    assert_omitted(
        "<p>kept</p><p id=\"r\">a <span style=\"position: relative; left: 5px\">moved</span></p>",
        "",
        "r",
    );
}

#[test]
fn transformed_paragraphs_have_no_runs_and_a_warning() {
    assert_omitted(
        "<p>kept</p><div style=\"transform: translateX(5px)\"><p id=\"t\">moved</p></div>",
        "",
        "t",
    );
}

#[test]
fn fixed_boxes_placed_by_other_insets_have_no_runs_and_a_warning() {
    assert_omitted(
        "<p>kept</p><div id=\"f\" style=\"position: fixed; bottom: 5px; right: 7px\">moved</div>",
        "",
        "f",
    );
}

#[test]
fn paragraphs_in_nested_columns_have_no_runs_and_a_warning() {
    assert_omitted(
        &format!(
            "<p>kept</p><div style=\"columns: 2\"><div style=\"columns: 2\">\
             <p id=\"n\">{}</p></div></div>",
            vec!["word"; 30].join(" ")
        ),
        "",
        "n",
    );
}

#[test]
fn relatively_positioned_blocks_have_no_runs_and_a_warning() {
    assert_omitted(
        "<p>kept</p><div style=\"position: relative; top: 5px\"><p id=\"b\">moved</p></div>",
        "",
        "b",
    );
}

#[test]
fn hyphenated_lines_match_paint() {
    let result = lay_out(
        "<p style=\"width: 60px; hyphens: manual\">aaaa\u{ad}bbbb\u{ad}cccc dd\u{ad}ee</p>",
        "",
    );
    check(&result);
    let page = result.pages().next().expect("page");
    assert!(
        page.text_runs()
            .iter()
            .any(|run| run.text.contains('\u{ad}'))
    );
}

#[test]
fn hidden_text_is_neither_painted_nor_reported() {
    let result = lay_out(
        "<p>shown</p><p style='visibility:hidden'>secret</p>\
         <p style='visibility:collapse'>folded</p>\
         <p style='visibility:hidden'><span style='visibility:visible'>seen</span></p>",
        "",
    );
    let page = result.pages().next().unwrap();
    let texts: Vec<&str> = page.text_runs().iter().map(|run| run.text).collect();
    assert_eq!(texts, ["shown", "seen"]);
    // Painted glyphs equal the reported ones: "shown" and "seen".
    assert_eq!(check(&result), 9);
}

/// The node whose `id` attribute is `id`.
fn by_id(page: &Page<'_>, id: &str) -> NodeId {
    let dom = page.dom();
    let mut stack = vec![dom.root()];
    while let Some(node) = stack.pop() {
        if dom.attr(node, "id") == Some(id) {
            return node;
        }
        stack.extend(dom.children(node));
    }
    panic!("no element with id {id}");
}

/// Bounding boxes `(x, y, width, height)` of every fill the painter draws.
fn filled(page: &Page<'_>, page_count: u32) -> Vec<(f64, f64, f64, f64)> {
    let (document, cascade, page_box, origin) = page.paint_inputs();
    let mut scene = Scene::new();
    let mut budget = CounterSnapshotBudget::default();
    paint_single_page_with_origin_and_page(
        &mut scene,
        document,
        cascade,
        page_box,
        origin,
        page.index(),
        page_count,
        page.index() % 2 == 1,
        &mut budget,
    )
    .expect("paint");
    scene
        .commands
        .iter()
        .filter_map(|command| match command {
            RenderCommand::Fill(fill) => {
                let bounds = kurbo::Shape::bounding_box(&(fill.transform * fill.shape.clone()));
                Some((bounds.x0, bounds.y0, bounds.width(), bounds.height()))
            }
            _ => None,
        })
        .collect()
}

#[test]
fn paint_rects_follow_the_body_margin_like_the_painter() {
    let result = lay_out(
        "<p id='flow'>flow</p>\
         <div id='abs' style='position: absolute; top: 120px; left: 30px; width: 50px; height: 12px; \
          background-color: rgb(255, 0, 0)'></div>",
        "body { margin: 20px } \
         p { margin: 0; width: 120px; height: 10px; padding: 4px; background-color: rgb(0, 0, 255) }",
    );
    let page = result.pages().next().unwrap();
    let fragment = |id: &str| {
        let node = by_id(&page, id);
        page.fragments()
            .find(|fragment| fragment.node() == node && fragment.kind() == FragmentKind::Box)
            .unwrap_or_else(|| panic!("no box fragment for {id}"))
    };
    let body = page
        .fragments()
        .find(|fragment| {
            fragment.kind() == FragmentKind::Box
                && page.dom().local_name(fragment.node()) == Some("body")
        })
        .expect("body fragment");
    let flow = fragment("flow");
    let abs = fragment("abs");

    // Layout already places the in-flow child inside the body's left margin,
    // while the absolute child keeps its offset from the page content box.
    let content = page.geometry().content_box;
    assert_eq!(body.paint_rect(), body.rect());
    assert_eq!(flow.paint_rect(), flow.rect());
    assert_eq!(abs.paint_rect(), abs.rect());
    assert_eq!(flow.rect().x, content.x + 20.0);
    assert_eq!(abs.rect().x, content.x + 30.0);

    // The painter fills both backgrounds exactly at their paint rects.
    let fills = filled(&page, result.page_count() as u32);
    for (name, fragment) in [("flow", flow), ("abs", abs)] {
        let r = fragment.paint_rect();
        let expected = (
            f64::from(r.x),
            f64::from(r.y),
            f64::from(r.width),
            f64::from(r.height),
        );
        assert!(
            fills.iter().any(|fill| {
                (fill.0 - expected.0).abs() < TOLERANCE
                    && (fill.1 - expected.1).abs() < TOLERANCE
                    && (fill.2 - expected.2).abs() < TOLERANCE
                    && (fill.3 - expected.3).abs() < TOLERANCE
            }),
            "{name}: no fill at {expected:?} among {fills:?}"
        );
    }

    // The paragraph's text starts at its paint rect's content edge.
    let run = page
        .text_runs()
        .into_iter()
        .find(|run| run.text == "flow")
        .expect("the paragraph's run");
    assert!((run.origin.0 - (flow.paint_rect().x + 4.0)).abs() < 1e-3);
}

#[test]
fn ellipsis_runs_match_paint_and_name_their_block() {
    let result = lay_out(
        "<p id=\"p\">aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa</p>",
        "#p { width: 100px; white-space: nowrap; overflow: hidden; \
         text-overflow: ellipsis; color: rgb(0, 0, 255) }",
    );
    assert_runs_match_paint(&result);
    let page = result.pages().next().expect("page");
    let runs = page.text_runs();
    let ellipsis = runs
        .iter()
        .find(|run| matches!(run.source, RunSource::Ellipsis(_)))
        .expect("an ellipsis run");
    let RunSource::Ellipsis(block) = ellipsis.source else {
        unreachable!();
    };
    assert_eq!(page.dom().attr(block, "id"), Some("p"));
    // Ahem has U+2026, so the ellipsis is one glyph.
    assert_eq!(ellipsis.text, "\u{2026}");
    assert_eq!(ellipsis.glyphs.len(), 1);
    assert_eq!(ellipsis.glyphs[0].text_range, 0..3);
    assert_eq!(
        (ellipsis.color.r, ellipsis.color.g, ellipsis.color.b),
        (0, 0, 255)
    );
    let text = runs
        .iter()
        .find(|run| matches!(run.source, RunSource::Text(_)))
        .expect("the kept text");
    assert!(text.origin.0 < ellipsis.origin.0);
}

#[test]
fn decoration_api_matches_the_builtin_painter_on_each_page() {
    use kurbo::Shape;
    let layout = lay_out(
        "<p>a<span style='font-size:6px;vertical-align:super'>b</span>c</p><p style='break-before:page'>de</p>",
        "p {text-decoration:underline overline line-through red;text-underline-offset:2px}",
    );
    assert_eq!(layout.page_count(), 2);
    for page in layout.pages() {
        let expected: Vec<_> = page
            .text_runs()
            .into_iter()
            .flat_map(|r| r.decorations)
            .collect();
        assert!(!expected.is_empty());
        let (document, cascade, page_box, origin) = page.paint_inputs();
        let mut scene = Scene::new();
        paint_single_page_with_origin_and_page(
            &mut scene,
            document,
            cascade,
            page_box,
            origin,
            page.index(),
            layout.page_count(),
            page.index() % 2 == 1,
            &mut CounterSnapshotBudget::default(),
        )
        .unwrap();
        let mut rectangles: Vec<_> = scene
            .commands
            .iter()
            .filter_map(|command| {
                let RenderCommand::Fill(fill) = command else {
                    return None;
                };
                (fill.brush == anyrender::Paint::Solid(peniko::Color::from_rgba8(255, 0, 0, 255)))
                    .then(|| {
                        fill.transform
                            .transform_rect_bbox(fill.shape.bounding_box())
                    })
            })
            .collect();
        for line in expected {
            let center = f64::from(line.y);
            let matching = rectangles.iter().position(|rect| {
                (rect.x0 - f64::from(line.x_start)).abs() < TOLERANCE
                    && (rect.x1 - f64::from(line.x_end)).abs() < TOLERANCE
                    && ((rect.y0 + rect.y1) * 0.5 - center).abs() < TOLERANCE
                    && (rect.height() - f64::from(line.thickness)).abs() < TOLERANCE
            });
            assert!(
                matching.is_some(),
                "page {}: missing {line:?}; painted {rectangles:?}",
                page.index()
            );
            rectangles.swap_remove(matching.unwrap());
        }
    }
}

#[test]
fn line_identity_groups_split_runs_but_distinguishes_coincident_lines_and_paragraphs() {
    let layout = lay_out(
        "<p>A<span style='color:red'>B</span>CD</p><p>EF</p>",
        "p {width:20px;margin:0;font:10px/0 Ahem;word-break:break-all}",
    );
    let runs = layout.page(0).unwrap().text_runs();
    let run = |text| runs.iter().find(|run| run.text == text).unwrap();
    let (a, b, cd, ef) = (run("A"), run("B"), run("CD"), run("EF"));
    assert_eq!(a.line, b.line);
    assert_eq!(a.line.root, cd.line.root);
    assert_eq!(cd.line.index, a.line.index + 1);
    assert_ne!(a.line.root, ef.line.root);
    assert_eq!(a.origin.1, cd.origin.1);
}
