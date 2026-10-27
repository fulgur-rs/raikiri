//! External-consumer coverage for page-margin boxes that show a named string
//! (`content: string(<name>, <fetch>)`, CSS GCPM 3 §1.1.2).

use raikiri_html::{
    DocumentLayout, FontCollectionBuilder, LayoutOptions, LayoutStatus, Page, PageMarginBoxSlot,
    RenderResources, layout, parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

const AHEM: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../raikiri-dom/tests/data/text-autospace/Ahem.ttf"
));

fn lay_out(margin_boxes: &str, body: &str) -> DocumentLayout {
    let html = format!(
        r#"<!doctype html><style>
          @page {{ size: 300px 200px; margin: 40px; {margin_boxes} }}
          body {{ margin: 0; font: 10px/10px Ahem }}
          p, h2 {{ margin: 0 }}
          h2 {{ string-set: title content(text) }}
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

/// The text of the margin box in `slot` of every page.
fn slot_texts(result: &DocumentLayout, slot: PageMarginBoxSlot) -> Vec<String> {
    result
        .pages()
        .map(|page: Page<'_>| {
            page.margin_boxes()
                .into_iter()
                .find(|margin_box| margin_box.slot == slot)
                .map(|margin_box| margin_box.content)
                .unwrap_or_default()
        })
        .collect()
}

/// The text of the top-left margin box of every page.
fn top_left(result: &DocumentLayout) -> Vec<String> {
    slot_texts(result, PageMarginBoxSlot::TopLeft)
}

/// Three pages: a heading with text on the first, a page with no heading,
/// and a heading on the third.
const THREE_PAGES: &str = r#"<h2>Introduction</h2><p>A</p>
    <p style="break-before: page">B</p>
    <h2 style="break-before: page">Background</h2><p>C</p>"#;

#[test]
fn a_named_string_takes_its_value_on_each_page_and_carries_it_forward() {
    let result = lay_out("@top-left { content: string(title) }", THREE_PAGES);
    assert_eq!(
        top_left(&result),
        ["Introduction", "Introduction", "Background"]
    );
}

/// Five pages: two headings on the first, none on the second, a heading
/// that starts the third and one below it, a heading in the middle of the
/// fourth, and none on the fifth.
const FIVE_PAGES: &str = r#"<h2>A1</h2><p>x</p><h2>A2</h2><p>x</p>
    <p style="break-before: page">y</p>
    <h2 style="break-before: page">C1</h2><p>z</p><h2>C2</h2><p>z</p>
    <p style="break-before: page">w</p><h2>D1</h2>
    <p style="break-before: page">v</p>"#;

#[test]
fn each_keyword_picks_among_the_assignments_of_the_page() {
    let result = lay_out(
        "@top-left { content: 'F:' string(title, first) } \
         @top-center { content: 'S:' string(title, start) } \
         @top-right { content: 'L:' string(title, last) } \
         @bottom-left { content: 'X:' string(title, first-except) }",
        FIVE_PAGES,
    );
    // A page without an assignment shows the entry value for every keyword,
    // `start` shows it too when the page opens with other content, and
    // `first-except` is empty on a page that has an assignment. WeasyPrint
    // draws the same strings.
    assert_eq!(
        slot_texts(&result, PageMarginBoxSlot::TopLeft),
        ["F:A1", "F:A2", "F:C1", "F:D1", "F:D1"]
    );
    assert_eq!(
        slot_texts(&result, PageMarginBoxSlot::TopCenter),
        ["S:A1", "S:A2", "S:C1", "S:C2", "S:D1"]
    );
    assert_eq!(
        slot_texts(&result, PageMarginBoxSlot::TopRight),
        ["L:A2", "L:A2", "L:C2", "L:D1", "L:D1"]
    );
    assert_eq!(
        slot_texts(&result, PageMarginBoxSlot::BottomLeft),
        ["X:", "X:A2", "X:", "X:", "X:D1"]
    );
}

#[test]
fn only_elements_with_a_box_assign_and_the_last_assignment_of_an_element_wins() {
    let result = lay_out(
        "@top-left { content: 'F:' string(title, first) } \
         @top-center { content: 'S:' string(title, start) } \
         @top-right { content: 'L:' string(title, last) } \
         @bottom-left { content: 'X:' string(title, first-except) } \
         @bottom-center { content: 'O:' string(other) }",
        r#"<h2>B1</h2><div style="display: none; string-set: title 'Hidden'">hidden</div><p>x</p>
           <p style="break-before: page">y<span style="string-set: title 'SPAN'">span</span>z</p>
           <p style="break-before: page">q</p>
           <div style="break-before: page; string-set: title 'T1', other 'O1', title 'T2'">two</div>
           <p style="break-before: page">end</p>"#,
    );
    // The `display: none` element assigns nothing. An inline element assigns
    // in the middle of the second page, so `start` keeps the value carried in,
    // and the two assignments of one element give its later value.
    assert_eq!(
        slot_texts(&result, PageMarginBoxSlot::TopLeft),
        ["F:B1", "F:SPAN", "F:SPAN", "F:T2", "F:T2"]
    );
    assert_eq!(
        slot_texts(&result, PageMarginBoxSlot::TopCenter),
        ["S:B1", "S:B1", "S:SPAN", "S:T2", "S:T2"]
    );
    assert_eq!(
        slot_texts(&result, PageMarginBoxSlot::TopRight),
        ["L:B1", "L:SPAN", "L:SPAN", "L:T2", "L:T2"]
    );
    assert_eq!(
        slot_texts(&result, PageMarginBoxSlot::BottomLeft),
        ["X:", "X:", "X:SPAN", "X:", "X:T2"]
    );
    // Another name keeps its own entry value.
    assert_eq!(
        slot_texts(&result, PageMarginBoxSlot::BottomCenter),
        ["O:", "O:", "O:", "O:O1", "O:O1"]
    );
}
