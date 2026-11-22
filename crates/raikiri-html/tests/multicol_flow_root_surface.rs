//! Where the lines inside a `display: flow-root` box stand in a multicol
//! container. A column end breaks such a box between the boxes it holds, as
//! it breaks any block (CSS Fragmentation 3, 4.4); the box only starts a
//! formatting context of its own, so the margins of what it holds do not
//! collapse with its own, and a `column-span: all` box inside it spans
//! nothing. The positions are the ones Chromium gives the same documents.

use raikiri_html::{
    DocumentLayout, FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

/// Ahem's ascent is 0.8em, which puts the top of a 10px glyph 8px above the
/// baseline.
const ASCENT: f32 = 8.0;
/// The advance of one Ahem glyph at 10px.
const ADVANCE: f32 = 10.0;

/// Every word of `body` as `page:word@x,y`, the left and top of its glyphs,
/// in source order, on 100x100 pages with 10px Ahem lines 10px apart. `.m`
/// is a two-column container 100px wide with no gap.
fn words(body: &str) -> String {
    let result = lay_out(body);
    let mut words = Vec::new();
    let mut index = 0;
    while let Some(page) = result.page(index) {
        for run in page.text_runs() {
            let mut x = run.origin.0;
            let y = (run.origin.1 - ASCENT).round();
            for word in run.text.split(' ') {
                if !word.trim().is_empty() {
                    words.push(format!("{index}:{}@{},{}", word.trim(), x.round(), y));
                }
                x += ADVANCE * (word.chars().count() + 1) as f32;
            }
        }
        index += 1;
    }
    words.join(" ")
}

fn lay_out(body: &str) -> DocumentLayout {
    let fonts = FontCollectionBuilder::new()
        .font_bytes(
            "Ahem",
            include_bytes!("../../raikiri-dom/tests/data/text-autospace/Ahem.ttf").to_vec(),
        )
        .build()
        .unwrap();
    let resources = RenderResources::new().fonts(fonts);
    let html = format!(
        "<!doctype html><style>@page{{size:100px 100px;margin:0}}\
         body{{margin:0;font:10px/10px Ahem}}.m{{columns:2;column-gap:0;width:100px}}\
         .f{{display:flow-root}}</style><body>{body}</body>"
    );
    let document = parse_html_with_resources(html.as_bytes(), &resources).unwrap();
    let LayoutStatus::Completed(result) = layout(
        &document,
        PageDefaults::default(),
        LayoutConfig::default(),
        LayoutOptions::new().resources(&resources),
    )
    .unwrap() else {
        panic!("layout must complete")
    };
    result
}

#[test]
fn a_flow_root_box_breaks_between_the_boxes_it_holds() {
    assert_eq!(
        words("<div class=m><div class=f><div style='height:10px'></div><div>C</div></div></div>"),
        "0:C@50,0"
    );
    assert_eq!(
        words(
            "<div class=m><div class=f><div>A</div><div>B</div><div>C</div><div>D</div></div></div>"
        ),
        "0:A@0,0 0:B@0,10 0:C@50,0 0:D@50,10"
    );
    // Flow-root boxes side by side and nested.
    assert_eq!(
        words(
            "<div class=m><div class=f><div class=f><div>A</div><div>B</div></div>\
             <div>C</div><div>D</div></div></div>"
        ),
        "0:A@0,0 0:B@0,10 0:C@50,0 0:D@50,10"
    );
    assert_eq!(
        words(
            "<div class=m><div class=f><div>A</div><div>B</div><div>C</div></div>\
             <div class=f><div>D</div><div>E</div><div>F</div></div></div>"
        ),
        "0:A@0,0 0:B@0,10 0:C@0,20 0:D@50,0 0:E@50,10 0:F@50,20"
    );
}

#[test]
fn a_flow_root_box_of_one_paragraph_breaks_between_its_lines() {
    assert_eq!(
        words("<div class=m><div class=f><div>A<br>B<br>C<br>D</div></div></div>"),
        "0:A@0,0 0:B@0,10 0:C@50,0 0:D@50,10"
    );
}

#[test]
fn margins_inside_a_flow_root_box_do_not_collapse_with_its_own() {
    // The margin of the first box adds to the box's own, so the box moves
    // whole to the next column rather than leave its first line alone.
    assert_eq!(
        words(
            "<div class=m><div>X</div><div class=f style='margin-top:10px'>\
             <div style='margin-top:10px'>A</div><div>B</div><div>C</div></div></div>"
        ),
        "0:X@0,0 0:A@0,30 0:B@50,0 0:C@50,10"
    );
    assert_eq!(
        words(
            "<div class=m><div style='margin-bottom:10px'>X</div><div class=f>\
             <div style='margin-top:10px'>A</div><div>B</div><div>C</div></div></div>"
        ),
        "0:X@0,0 0:A@0,30 0:B@50,0 0:C@50,10"
    );
    // The margin of the last box stays inside the box, and the end of the
    // column cuts it with the margin after the box.
    assert_eq!(
        words(
            "<div class=m><div class=f><div>A</div><div style='margin-bottom:20px'>B</div></div>\
             <div style='margin-top:10px'>C</div><div>D</div></div>"
        ),
        "0:A@0,0 0:B@0,10 0:C@50,0 0:D@50,10"
    );
    assert_eq!(
        words(
            "<div class=m><div class=f><div>A</div><div>B</div>\
             <div style='margin-bottom:20px'>C</div></div><div>Z</div></div>"
        ),
        "0:A@0,0 0:B@0,10 0:C@0,20 0:Z@50,0"
    );
    // A box that a column end moves keeps the margin of its first box.
    assert_eq!(
        words(
            "<div class=m><div>X</div><div>Y</div><div class=f style='margin-top:20px'>\
             <div style='margin-top:10px'>A</div><div>B</div></div><div>C</div><div>D</div></div>"
        ),
        "0:X@0,0 0:Y@0,10 0:A@50,10 0:B@50,20 0:C@50,30 0:D@50,40"
    );
}

#[test]
fn a_flow_root_box_breaks_in_columns_of_a_set_height() {
    assert_eq!(
        words(
            "<div class=m style='height:20px;column-fill:auto'><div class=f>\
             <div>A</div><div>B</div><div>C</div><div>D</div></div></div>"
        ),
        "0:A@0,0 0:B@0,10 0:C@50,0 0:D@50,10"
    );
    assert_eq!(
        words(
            "<div class=m style='height:30px;column-fill:auto'><div>X</div><div>Y</div>\
             <div style='margin-bottom:20px'>W</div><div class=f>\
             <div style='margin-top:10px'>A</div><div>B</div></div></div>"
        ),
        "0:X@0,0 0:Y@0,10 0:W@0,20 0:A@50,10 0:B@50,20"
    );
    // Content past the last column goes on in overflow columns.
    assert_eq!(
        words(
            "<div class=m style='height:20px;column-fill:auto'><div class=f><div>A</div>\
             <div>B</div><div>C</div><div style='margin-bottom:20px'>D</div></div>\
             <div>Z</div></div>"
        ),
        "0:A@0,0 0:B@0,10 0:C@50,0 0:D@50,10 0:Z@100,0"
    );
}

#[test]
fn a_column_spanning_box_inside_a_flow_root_box_spans_nothing() {
    assert_eq!(
        words(
            "<div class=m><div class=f><div>A</div><div style='column-span:all'>S</div>\
             <div>B</div><div>C</div></div></div>"
        ),
        "0:A@0,0 0:S@0,10 0:B@50,0 0:C@50,10"
    );
    // One in a plain block that is not inside a flow-root box still spans.
    assert_eq!(
        words(
            "<div class=m><div>X</div><div><div style='column-span:all'>S</div></div>\
             <div>B</div><div>C</div></div>"
        ),
        "0:X@0,0 0:S@0,10 0:B@0,20 0:C@50,20"
    );
}

#[test]
fn a_flow_root_box_of_one_line_inside_a_wrapper_goes_to_one_column() {
    assert_eq!(
        words(
            "<div class=m><div><div class=f><div>A</div></div><div>B</div><div>C</div></div></div>"
        ),
        "0:A@0,0 0:B@0,10 0:C@50,0"
    );
}

#[test]
fn the_last_line_of_a_flow_root_box_goes_with_the_padding_below_it() {
    assert_eq!(
        words(
            "<div class=m><div style='padding-bottom:10px'><div class=f><div>A</div>\
             <div>B</div><div>C</div></div></div></div>"
        ),
        "0:A@0,0 0:B@0,10 0:C@50,0"
    );
    assert_eq!(
        words(
            "<div class=m><div style='padding-bottom:20px'><div class=f><div>A</div>\
             <div>B</div><div>C</div><div>D</div></div></div></div>"
        ),
        "0:A@0,0 0:B@0,10 0:C@0,20 0:D@50,0"
    );
}

#[test]
fn a_box_that_starts_a_formatting_context_keeps_the_margins_inside_apart() {
    // A size container starts one too.
    assert_eq!(
        words(
            "<div class=m><div style='container-type:inline-size'>\
             <div style='margin-top:10px'>A</div><div>B</div><div>C</div></div></div>"
        ),
        "0:A@0,10 0:B@50,0 0:C@50,10"
    );
    // A box whose first lines the next column takes keeps the margin above
    // them.
    assert_eq!(
        words(
            "<div class=m><div>X</div><div>Y</div><div>W</div>\
             <div class=f style='margin-top:5px'><div style='margin-top:10px;orphans:3'>\
             A<br>B<br>C</div></div></div>"
        ),
        "0:X@0,0 0:Y@0,10 0:W@0,20 0:A@50,10 0:B@50,20 0:C@50,30"
    );
}
