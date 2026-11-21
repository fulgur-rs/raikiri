//! Where the lines of a `position: relative` paragraph inside a wrapper go
//! in a multicol container with a definite height when the paragraph breaks
//! between the columns: the offset moves the lines in every column, those in
//! a later one from the top of that column. The positions are the ones
//! Chromium prints for the same documents.
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
fn a_relative_paragraph_keeps_its_offset_in_every_column() {
    let cases = [
        // A relative paragraph inside a plain wrapper.
        (
            "<div><div style='position:relative;top:5px'>S<br>T<br>U<br>V</div></div>",
            "S@0:0,5 T@0:0,15 U@0:40,5 V@0:40,15",
        ),
        // Inline and block insets.
        (
            "<div><div style='position:relative;left:10px;top:5px'>S<br>T<br>U<br>V</div></div>",
            "S@0:10,5 T@0:10,15 U@0:50,5 V@0:50,15",
        ),
        // Deeper inside plain wrappers.
        (
            "<div><div><div style='position:relative;top:5px'>S<br>T<br>U<br>V</div></div></div>",
            "S@0:0,5 T@0:0,15 U@0:40,5 V@0:40,15",
        ),
        // A relative wrapper around the paragraph.
        (
            "<div><div style='position:relative;top:5px'><div>S<br>T<br>U<br>V</div></div></div>",
            "S@0:0,5 T@0:0,15 U@0:40,5 V@0:40,15",
        ),
        // An offset up the page.
        (
            "<div><div style='position:relative;bottom:5px'>S<br>T<br>U<br>V</div></div>",
            "S@0:0,-5 T@0:0,5 U@0:40,-5 V@0:40,5",
        ),
        // Offsets of nested boxes add up.
        (
            "<div><div style='position:relative;top:5px'><div style='position:relative;top:3px'>S<br>T<br>U<br>V</div></div></div>",
            "S@0:0,8 T@0:0,18 U@0:40,8 V@0:40,18",
        ),
    ];
    for (wrapper, expected) in cases {
        assert_eq!(in_columns(wrapper), expected, "{wrapper}");
    }
}
