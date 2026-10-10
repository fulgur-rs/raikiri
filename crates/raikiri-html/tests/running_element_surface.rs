//! External-consumer coverage for running elements (CSS GCPM 3 §1.2): the
//! element a page margin box shows on each page, and its layout at the
//! margin box's width.

use raikiri_html::{
    DocumentLayout, FontCollectionBuilder, LayoutOptions, LayoutStatus, NodeId, RenderResources,
    StringFetchMode, layout, parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

const AHEM: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../raikiri-dom/tests/data/text-autospace/Ahem.ttf"
));

fn lay_out(body: &str) -> DocumentLayout {
    let html = format!(
        r#"<!doctype html><style>
          @page {{ size: 300px 200px; margin: 40px; @top-center {{ content: element(hdr) }} }}
          body {{ margin: 0; font: 10px/10px Ahem }}
          p {{ margin: 0 }}
          .hdr {{ position: running(hdr) }}
        </style>{body}"#
    );
    let fonts = FontCollectionBuilder::new()
        .font_bytes("Ahem", AHEM.to_vec())
        .build()
        .expect("fonts");
    let resources = RenderResources::new().fonts(fonts);
    let doc = parse_html_with_resources(html.as_bytes(), &resources).expect("parse");
    let LayoutStatus::Completed(result) = layout(
        &doc,
        PageDefaults::default(),
        LayoutConfig::default(),
        LayoutOptions::new().resources(&resources),
    )
    .expect("layout") else {
        panic!("expected a complete layout");
    };
    result
}

fn texts(page: &raikiri_html::Page<'_>) -> Vec<String> {
    page.text_runs()
        .iter()
        .map(|run| run.text.to_string())
        .collect()
}

fn id_of(result: &DocumentLayout, node: NodeId) -> Option<String> {
    let page = result.pages().next()?;
    page.dom().attr(node, "id").map(str::to_owned)
}

#[test]
fn running_elements_leave_the_page_flow() {
    let result = lay_out(r#"<div class="hdr">Head</div><p>Body</p>"#);
    let page = result.pages().next().expect("one page");
    assert_eq!(texts(&page), vec!["Body"]);
    // The paragraph is at the top of the content box: the running element
    // takes no space where it stands.
    let body = page.text_runs()[0].origin;
    assert_eq!(body, (40.0, 48.0));
}

#[test]
fn each_page_selects_its_running_element_by_keyword() {
    let result = lay_out(
        r#"<div class="hdr" id="a">A</div><p>one</p>
           <p style="break-before: page">two</p>
           <div class="hdr" id="b">B</div><div class="hdr" id="c">C</div><p>more</p>
           <p style="break-before: page">three</p>"#,
    );
    assert_eq!(result.page_count(), 3);
    let pick = |page: u32, fetch| {
        let page = result.page(page).expect("page");
        page.running_element("hdr", fetch)
            .and_then(|node| id_of(&result, node))
    };
    let first = StringFetchMode::First;
    let start = StringFetchMode::Start;
    let last = StringFetchMode::Last;
    let except = StringFetchMode::FirstExcept;
    let some = |id: &str| Some(id.to_owned());

    assert_eq!(pick(0, first), some("a"));
    assert_eq!(pick(0, start), some("a"));
    assert_eq!(pick(0, except), None);
    // B and C follow "two" on page 1, so neither starts that page.
    assert_eq!(pick(1, first), some("b"));
    assert_eq!(pick(1, start), some("a"));
    assert_eq!(pick(1, last), some("c"));
    assert_eq!(pick(1, except), None);
    // Page 2 has no assignment: the last earlier one carries over.
    for fetch in [first, start, last, except] {
        assert_eq!(pick(2, fetch), some("c"));
    }
    assert_eq!(result.page(0).unwrap().running_element("ftr", first), None);
}

#[test]
fn a_running_element_at_the_top_of_a_forced_page_starts_that_page() {
    let result = lay_out(
        r#"<div class="hdr" id="a">A</div><p>one</p>
           <section style="break-before: page"><div class="hdr" id="b">B</div><p>two</p></section>"#,
    );
    let page = result.page(1).expect("second page");
    let start = page
        .running_element("hdr", StringFetchMode::Start)
        .expect("start");
    assert_eq!(id_of(&result, start).as_deref(), Some("b"));
}

#[test]
fn the_selected_element_lays_out_at_the_margin_box_width() {
    let result = lay_out(
        r#"<section style="padding: 20px; border: 5px solid; font-size: 20px; line-height: 20px">
             <div class="hdr" style="margin: 3px 0 7px; padding: 2px; background: red">
               Running header text
             </div>
           </section><p>Body</p>"#,
    );
    let page = result.pages().next().expect("page");
    let node = page
        .running_element("hdr", StringFetchMode::First)
        .expect("selected");
    let running = result
        .layout_running_element(node, 200.0)
        .expect("layout")
        .expect("a running element");
    assert_eq!(running.node(), node);
    assert_eq!(running.width(), 200.0);
    let laid_out = running.page();
    // The font comes from the element's place in the document (20px from
    // the section); the section's own padding and border do not apply.
    // 196px of content fit 9 Ahem glyphs a line, so the text wraps after
    // each word.
    let runs = laid_out.text_runs();
    let lines: Vec<_> = runs
        .iter()
        .map(|run| (run.text.trim_end().to_owned(), run.origin))
        .collect();
    assert_eq!(
        lines,
        vec![
            ("Running".to_owned(), (2.0, 21.0)),
            ("header".to_owned(), (2.0, 41.0)),
            ("text".to_owned(), (2.0, 61.0)),
        ]
    );
    // 3px top margin + 2px padding + 3 lines + 2px padding + 7px bottom margin.
    assert_eq!(running.height(), 3.0 + 2.0 + 60.0 + 2.0 + 7.0);
    let element = laid_out
        .fragments()
        .find(|fragment| fragment.node() == node)
        .expect("the element's box");
    let rect = element.paint_rect();
    assert_eq!(
        (rect.x, rect.y, rect.width, rect.height),
        (0.0, 3.0, 200.0, 64.0)
    );
    assert_eq!(laid_out.geometry().page_box.height, running.height());
}

#[test]
fn the_display_of_a_running_element_applies_in_its_layout() {
    let result = lay_out(
        r#"<div class="hdr" style="display: flex; justify-content: space-between">
             <span>L</span><span>R</span>
           </div><p>Body</p>"#,
    );
    let page = result.pages().next().expect("page");
    let node = page
        .running_element("hdr", StringFetchMode::First)
        .expect("selected");
    let running = result
        .layout_running_element(node, 100.0)
        .expect("layout")
        .expect("a running element");
    let origins: Vec<_> = running
        .page()
        .text_runs()
        .iter()
        .map(|run| (run.text.to_string(), run.origin.0))
        .collect();
    assert_eq!(origins, vec![("L".to_owned(), 0.0), ("R".to_owned(), 90.0)]);
}

#[test]
fn only_running_elements_have_a_running_layout() {
    let result = lay_out(r#"<div class="hdr">Head</div><p id="p">Body</p>"#);
    let page = result.pages().next().expect("page");
    let paragraph = page
        .fragments()
        .map(|fragment| fragment.node())
        .find(|&node| page.dom().attr(node, "id") == Some("p"))
        .expect("paragraph");
    assert!(
        result
            .layout_running_element(paragraph, 100.0)
            .expect("layout")
            .is_none()
    );
    assert!(
        result
            .layout_running_element(NodeId::new(u64::MAX), 100.0)
            .expect("layout")
            .is_none()
    );
}

#[test]
fn a_running_element_after_all_content_is_assigned_on_the_last_page() {
    let result = lay_out(
        r#"<!-- note --><p>one</p><p style="break-before: page">two</p>
           <div class="hdr" id="tail">Tail</div>"#,
    );
    assert_eq!(result.page_count(), 2);
    let pick = |page: u32| {
        result
            .page(page)
            .expect("page")
            .running_element("hdr", StringFetchMode::First)
            .and_then(|node| id_of(&result, node))
    };
    assert_eq!(pick(0), None);
    assert_eq!(pick(1).as_deref(), Some("tail"));
}

#[test]
fn margins_in_percent_and_auto_and_unusable_widths_are_handled() {
    let result = lay_out(
        r#"<div class="hdr" id="pct" style="margin-bottom: 10%">P</div>
           <div class="hdr" id="auto" style="margin-bottom: auto">A</div><p>Body</p>"#,
    );
    let page = result.pages().next().expect("page");
    let first = page
        .running_element("hdr", StringFetchMode::First)
        .expect("first");
    let last = page
        .running_element("hdr", StringFetchMode::Last)
        .expect("last");
    let pct = result
        .layout_running_element(first, 100.0)
        .expect("layout")
        .expect("running");
    // One 10px line and a 10% (of 100px) bottom margin.
    assert_eq!(pct.height(), 20.0);
    let auto = result
        .layout_running_element(last, 100.0)
        .expect("layout")
        .expect("running");
    assert_eq!(auto.height(), 10.0);
    let unusable = result
        .layout_running_element(first, f32::NAN)
        .expect("layout")
        .expect("running");
    assert_eq!(unusable.width(), 0.0);
}

#[test]
fn a_running_layout_is_made_once_per_element_and_width() {
    let result =
        lay_out(r#"<div class="hdr" id="a">A</div><div class="hdr" id="b">B</div><p>Body</p>"#);
    let page = result.pages().next().expect("page");
    let first = page
        .running_element("hdr", StringFetchMode::First)
        .expect("first");
    let last = page
        .running_element("hdr", StringFetchMode::Last)
        .expect("last");
    let layout = |node, width| {
        result
            .layout_running_element(node, width)
            .expect("layout")
            .expect("running")
    };
    let shared = layout(first, 100.0);
    assert!(std::ptr::eq(shared, layout(first, 100.0)));
    assert!(!std::ptr::eq(shared, layout(first, 50.0)));
    assert!(!std::ptr::eq(shared, layout(last, 100.0)));
    // Widths that cannot be used all lay out at 0.
    let zero = layout(first, 0.0);
    for unusable in [-5.0, -0.0, f32::NAN, f32::INFINITY] {
        assert!(std::ptr::eq(zero, layout(first, unusable)));
    }
}

#[test]
fn a_document_without_running_elements_selects_nothing() {
    let html = br#"<!doctype html><style>@page { size: 300px 200px }</style><p id="p">Body</p>"#;
    let resources = RenderResources::new();
    let doc = parse_html_with_resources(&html[..], &resources).expect("parse");
    let LayoutStatus::Completed(result) = layout(
        &doc,
        PageDefaults::default(),
        LayoutConfig::default(),
        LayoutOptions::new().resources(&resources),
    )
    .expect("layout") else {
        panic!("expected a complete layout");
    };
    let page = result.pages().next().expect("page");
    assert_eq!(page.running_element("hdr", StringFetchMode::First), None);
    let node = page.fragments().next().expect("a fragment").node();
    assert!(
        result
            .layout_running_element(node, 100.0)
            .expect("layout")
            .is_none()
    );
}
