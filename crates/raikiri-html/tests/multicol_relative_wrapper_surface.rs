//! Where the boxes inside a `position: relative` wrapper go in a multicol
//! container with a definite height: the wrapper breaks between the columns
//! as a plain one does, and its offset moves every piece of it. The positions
//! are the ones Chromium prints for the same documents.
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

/// `body` laid out on 100x100 pages with no margin and 10px Ahem lines 10px
/// apart.
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
         body{{margin:0;font:10px/10px Ahem}}</style><body>{body}</body>"
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

/// Every word of `body` as `word@page:x,y`, the left and top of its glyphs
/// where they are painted on their page.
fn words(body: &str) -> String {
    let result = lay_out(body);
    let mut words = Vec::new();
    for index in 0..result.page_count() {
        let page = result.page(index).expect("a page");
        for run in page.text_runs() {
            let origin = run
                .transform
                .map_or(run.origin, |transform| transform.apply(run.origin));
            let mut x = origin.0;
            let y = (origin.1 - ASCENT).round();
            for word in run.text.split(' ') {
                if !word.trim().is_empty() {
                    words.push(format!("{}@{index}:{},{}", word.trim(), x.round(), y));
                }
                x += ADVANCE * (word.chars().count() + 1) as f32;
            }
        }
    }
    words.sort();
    words.join(" ")
}

/// `wrapper` in two 40px columns 30px tall with no gap.
fn in_columns(wrapper: &str) -> String {
    words(&format!(
        "<div style='columns:2;height:30px;column-fill:auto;column-gap:0;width:80px'>{wrapper}</div>"
    ))
}

#[test]
fn a_relative_wrapper_breaks_between_the_columns() {
    let cases = [
        // The wrapper moves down with its pieces in both columns.
        (
            "<div style='position:relative;top:50px'><div>S</div><div>T</div><div>U</div><div>V</div></div>",
            "S@0:0,50 T@0:0,60 U@0:0,70 V@0:40,50",
        ),
        // Inline and block insets on the wrapper.
        (
            "<div style='position:relative;left:10px;top:5px'><div>S</div><div>T</div><div>U</div><div>V</div></div>",
            "S@0:10,5 T@0:10,15 U@0:10,25 V@0:50,5",
        ),
        // A relative wrapper inside a plain one keeps its inline offset.
        (
            "<div><div style='position:relative;left:10px;top:5px'><div>S</div><div>T</div><div>U</div><div>V</div></div></div>",
            "S@0:10,5 T@0:10,15 U@0:10,25 V@0:50,5",
        ),
        // Offsets of nested wrappers add up.
        (
            "<div style='position:relative;top:5px'><div style='position:relative;left:10px;top:5px'><div>S</div><div>T</div></div><div>U</div><div>V</div></div>",
            "S@0:10,10 T@0:10,20 U@0:0,25 V@0:40,5",
        ),
        // Padding above the first box still takes room in the first column.
        (
            "<div style='position:relative;top:5px;padding-top:5px'><div>S</div><div>T</div><div>U</div><div>V</div></div>",
            "S@0:0,10 T@0:0,20 U@0:40,5 V@0:40,15",
        ),
        // A relative paragraph inside a relative wrapper.
        (
            "<div style='position:relative;top:5px'><div style='position:relative;left:10px'>S</div><div>T</div><div>U</div><div>V</div></div>",
            "S@0:10,5 T@0:0,15 U@0:0,25 V@0:40,5",
        ),
        // A percentage inset is a share of the wrapper's width.
        (
            "<div><div style='position:relative;left:50%'>S</div><div>T</div><div>U</div><div>V</div></div>",
            "S@0:20,0 T@0:0,10 U@0:0,20 V@0:40,0",
        ),
        // In a right-to-left wrapper `right` wins over `left`.
        (
            "<div style='direction:rtl'><div style='position:relative;left:10px;right:5px'>S</div>\
             <div>T</div><div>U</div><div>V</div></div>",
            "S@0:25,0 T@0:30,10 U@0:30,20 V@0:70,0",
        ),
    ];
    for (wrapper, expected) in cases {
        assert_eq!(in_columns(wrapper), expected, "{wrapper}");
    }
}

#[test]
fn a_relative_wrapper_moved_past_the_page_end_breaks_on_the_next_page() {
    assert_eq!(
        words(
            "<div style='columns:2;height:30px;column-fill:auto'>\
             <div style='position:relative;top:150px'>\
             <div>S</div><div>T</div><div>U</div><div>V</div></div></div>"
        ),
        "S@1:0,50 T@1:0,60 U@1:0,70 V@1:55,50"
    );
}
