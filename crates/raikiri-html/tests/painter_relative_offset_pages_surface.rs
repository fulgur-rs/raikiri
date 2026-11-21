//! Which page shows a box that layout places without its relative or sticky
//! offset and the painter moves: a float, a table cell, row or table, a
//! sticky box and a box in columns. The box is printed where it is painted, so a block offset that
//! moves it past the page end puts it on a later page. The positions are the
//! ones Chromium prints for the same documents.
use raikiri_html::{
    DocumentLayout, FontCollectionBuilder, FragmentKind, LayoutOptions, LayoutStatus,
    RenderResources, layout, parse_html_with_resources,
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

#[test]
fn a_box_moved_past_the_page_end_is_printed_on_the_next_page() {
    let cases = [
        (
            "<div style='float:left;width:50px;height:20px;position:relative;top:150px'>S</div>",
            "S@1:0,50",
        ),
        (
            "<div style='float:left;width:50px;position:relative;top:150px'>S<br>T<br>U</div>",
            "S@1:0,50 T@1:0,60 U@1:0,70",
        ),
        (
            "<table><tr><td style='position:relative;top:150px'>S<br>T</td></tr></table>",
            "S@1:3,53 T@1:3,63",
        ),
        (
            "<table><tr style='position:relative;top:150px'><td>S</td></tr></table>",
            "S@1:3,53",
        ),
        (
            "<table style='position:relative;top:150px'><tr><td>S</td></tr></table>",
            "S@1:3,53",
        ),
        (
            "<div style='columns:2'><div style='height:20px;position:relative;top:150px'>S</div></div>",
            "S@1:0,50",
        ),
        (
            "<div style='columns:2;width:100px'>\
             <div style='column-span:all;position:relative;top:150px'>S</div></div>",
            "S@1:0,50",
        ),
        (
            "<div style='float:left;width:50px;position:relative;top:150px;columns:2'>S T</div>",
            "S@1:0,50 T@1:0,60",
        ),
        (
            "<div style='float:left;width:50px;position:relative;top:150px'>\
             <span style='display:inline-block;width:10px;height:10px'>S</span></div>",
            "S@1:0,50",
        ),
        (
            "<div style='height:250px'><div style='position:sticky;top:150px;height:20px'>S</div></div>",
            "S@1:0,50",
        ),
    ];
    for (body, expected) in cases {
        assert_eq!(words(body), expected, "{body}");
    }
}

#[test]
fn offsets_add_up_and_split_lines_at_the_painted_page_edge() {
    let cases = [
        // A float's lines go to the pages they are painted on.
        (
            "<div style='float:left;width:50px;position:relative;top:80px'>S<br>T<br>U<br>V</div>",
            "S@0:0,80 T@0:0,90 U@1:0,0 V@1:0,10",
        ),
        // A relative box in a moved float moves with it.
        (
            "<div style='float:left;width:50px;position:relative;top:150px'>\
             <div style='position:relative;top:100px'>S</div></div>",
            "S@2:0,50",
        ),
        // An offset back up from a later page.
        (
            "<div style='height:250px'>A</div>\
             <div style='float:left;width:50px;height:20px;position:relative;top:-150px'>S</div>",
            "A@0:0,0 S@1:0,0",
        ),
        // An offset that stays on the page changes nothing.
        (
            "<div style='float:left;width:50px;height:20px;position:relative;top:50px'>S</div>",
            "S@0:0,50",
        ),
        (
            "<table><tr><td style='position:relative;top:50px'>S</td></tr></table>",
            "S@0:3,53",
        ),
        (
            "<div style='columns:2'><div style='height:20px;position:relative;top:95px'>S</div></div>",
            "S@1:0,-5",
        ),
    ];
    for (body, expected) in cases {
        assert_eq!(words(body), expected, "{body}");
    }
}

/// The page and line range of every text fragment of `body`.
fn text_fragments(body: &str) -> Vec<(u32, Option<std::ops::Range<u32>>)> {
    lay_out(body)
        .pages()
        .flat_map(|page| {
            let index = page.index();
            page.fragments()
                .filter(|fragment| fragment.kind() == FragmentKind::Text)
                .map(|fragment| (index, fragment.line_range()))
                .collect::<Vec<_>>()
        })
        .collect()
}

#[test]
fn each_page_names_the_lines_painted_on_it() {
    assert_eq!(
        text_fragments(
            "<div style='float:left;width:10px;position:relative;top:80px'>S T U V</div>"
        ),
        [(0, Some(0..2)), (1, Some(2..4))]
    );
    // A paragraph in a vertical writing mode moves along with its float too.
    assert_eq!(
        text_fragments(
            "<div style='float:left;writing-mode:vertical-rl;height:20px;\
             position:relative;top:150px'>S</div><div style='height:300px'></div>"
        ),
        [(1, Some(0..1))]
    );
}
