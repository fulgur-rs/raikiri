//! Integration coverage for generated-content paint flow.

use anyrender::{Scene, recording::RenderCommand};
use raikiri_dom::{Document, layout_single_page};
use raikiri_paint::paint_single_page;
use raikiri_style::{build_rule_tree, cascade};
use raikiri_traits::PageBox;
use taffy::Style;

#[test]
fn nested_block_generated_counter_content_paints_in_flow() {
    let mut document = Document::new();
    let html = document.append_element(Some(0), "html", Style::default(), None::<&str>);
    let head = document.append_element(Some(html), "head", Style::default(), None::<&str>);
    let style = document.append_element(Some(head), "style", Style::default(), None::<&str>);
    document.append_text(
        style,
        r##"div { display: block; border: 2px solid black } div::before { content: counters(test, "."); counter-reset: test }"##,
    );
    let body = document.append_element(Some(html), "body", Style::default(), None::<&str>);
    let outer = document.append_element(Some(body), "div", Style::default(), None::<&str>);
    document.append_element(Some(outer), "div", Style::default(), None::<&str>);

    let rules = build_rule_tree(&document);
    let cascade = cascade(&document, &rules).expect("cascade Ok");
    layout_single_page(&mut document, &cascade, PageBox::A4).expect("layout Ok");

    let mut scene = Scene::new();
    paint_single_page(&mut scene, &document, &cascade, PageBox::A4);
    let glyph_runs = scene
        .commands
        .iter()
        .filter(|command| matches!(command, RenderCommand::GlyphRun(_)))
        .count();
    assert!(
        glyph_runs >= 2,
        "expected both nested counter pseudos to paint"
    );
}

#[test]
fn nested_absolute_generated_content_does_not_repeat_subtree_work() {
    const NESTED_BLOCKS: usize = 8;

    let mut document = Document::new();
    let html = document.append_element(Some(0), "html", Style::default(), None::<&str>);
    let head = document.append_element(Some(html), "head", Style::default(), None::<&str>);
    let style = document.append_element(Some(head), "style", Style::default(), None::<&str>);
    document.append_text(
        style,
        "div { display: block } div::before { content: \"x\"; position: absolute }",
    );
    let body = document.append_element(Some(html), "body", Style::default(), None::<&str>);
    let mut parent = body;
    for _ in 0..NESTED_BLOCKS {
        parent = document.append_element(Some(parent), "div", Style::default(), None::<&str>);
    }

    let rules = build_rule_tree(&document);
    let cascade = cascade(&document, &rules).expect("cascade Ok");
    let fonts = raikiri_dom::build_wpt_font_collection(std::path::Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../raikiri-dom/tests/data/text-autospace"
    )))
    .expect("Ahem font collection");
    document.set_font_collection(fonts);
    layout_single_page(&mut document, &cascade, PageBox::A4).expect("layout Ok");

    let calls_before_paint = document.standalone_text_calls();
    let mut scene = Scene::new();
    paint_single_page(&mut scene, &document, &cascade, PageBox::A4);
    let paint_text_calls = document.standalone_text_calls() - calls_before_paint;

    let glyph_runs = scene
        .commands
        .iter()
        .filter(|command| matches!(command, RenderCommand::GlyphRun(_)))
        .count();
    assert_eq!(glyph_runs, NESTED_BLOCKS);
    assert!(
        paint_text_calls <= 6 * NESTED_BLOCKS + 4,
        "painting {NESTED_BLOCKS} nested blocks used {paint_text_calls} standalone text measurements"
    );
}
