//! The bottom padding of a block whose flow ends inside a `display: contents`
//! element. The element lends its boxes to the flow of the block, so its last
//! box moves to the next page with the padding when the two do not fit, and
//! the margin that collapses through an empty box at the end is cut at the
//! page edge as in a block without the element. The positions are the ones
//! Chromium prints for the same documents.

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

/// Ahem's ascent is 0.8em, which puts the top of a 20px glyph 16px above the
/// baseline.
const ASCENT: f32 = 16.0;

/// The page, text and glyph top of every run of text.
fn runs(body: &str) -> Vec<(u32, String, f32)> {
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
         body{{margin:0;font:20px/30px Ahem}}</style><body>{body}</body>"
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
    let mut runs = Vec::new();
    for index in 0..result.page_count() {
        let Some(page) = result.page(index) else {
            continue;
        };
        for run in page.text_runs() {
            runs.push((index, run.text.trim_end().to_owned(), run.origin.1 - ASCENT));
        }
    }
    runs
}

/// `tail` at the end of a block with 15px of bottom padding, below four
/// lines that fill the first two pages up to 60px, and the block `Z` after
/// it.
fn ending(tail: &str) -> Vec<(u32, String, f32)> {
    runs(&format!(
        "<div style='width:20px;padding-bottom:15px'><div>X X X X</div>{tail}</div>\
         <div>Z</div>"
    ))
}

fn run(page: u32, text: &str, top: f32) -> (u32, String, f32) {
    (page, text.to_owned(), top)
}

/// The four lines of `X`, then `rest`.
fn lines_then(rest: &[(u32, &str, f32)]) -> Vec<(u32, String, f32)> {
    let mut runs = vec![
        run(0, "X", 5.0),
        run(0, "X", 35.0),
        run(1, "X", 5.0),
        run(1, "X", 35.0),
    ];
    runs.extend(rest.iter().map(|&(page, text, top)| run(page, text, top)));
    runs
}

#[test]
fn the_last_box_inside_contents_moves_with_the_padding() {
    // The 10px box `T` leaves 30px of the page, which holds `U` but not the
    // padding after it.
    let expected = lines_then(&[(1, "T", 65.0), (2, "U", 5.0), (2, "Z", 50.0)]);
    let t = "<div style='height:10px'>T</div>";
    let u = "<div style='height:30px'>U</div>";
    assert_eq!(ending(&format!("{t}{u}")), expected);
    assert_eq!(
        ending(&format!("{t}<div style='display:contents'>{u}</div>")),
        expected
    );
    assert_eq!(
        ending(&format!(
            "{t}<div style='display:contents'><div style='display:contents'>{u}</div></div>"
        )),
        expected
    );
    // Both boxes inside the element; a height limit on the element, which
    // generates no box, changes nothing.
    assert_eq!(
        ending(&format!("<div style='display:contents'>{t}{u}</div>")),
        expected
    );
    assert_eq!(
        ending(&format!(
            "{t}<div style='display:contents;min-height:10px'>{u}</div>"
        )),
        expected
    );
    // Its own margin comes along.
    assert_eq!(
        ending(&format!(
            "{t}<div style='display:contents'>\
             <div style='height:30px;margin-bottom:5px'>U</div></div>"
        )),
        lines_then(&[(1, "T", 65.0), (2, "U", 5.0), (2, "Z", 55.0)])
    );
}

#[test]
fn an_empty_box_inside_contents_moves_with_the_padding() {
    // With no margin above it, the empty box of height zero goes on with the
    // padding and keeps its 30px margin whole above it.
    let expected = lines_then(&[(1, "T", 65.0), (2, "Z", 50.0)]);
    let t = "<div style='height:20px'>T</div>";
    let empty = "<div style='height:0;margin-bottom:30px'></div>";
    assert_eq!(ending(&format!("{t}{empty}")), expected);
    assert_eq!(
        ending(&format!("{t}<div style='display:contents'>{empty}</div>")),
        expected
    );
}

#[test]
fn a_margin_through_an_empty_box_after_contents_is_cut_at_the_page_edge() {
    // The 5px margin below `T` collapses into the top of the empty box,
    // which then stays behind with it: the page edge cuts the margin, and
    // only the padding begins the next page.
    let expected = lines_then(&[(1, "T", 65.0), (2, "Z", 20.0)]);
    let t = "<div style='height:20px;margin-bottom:5px'>T</div>";
    let empty = "<div style='height:0;margin-bottom:30px'></div>";
    assert_eq!(ending(&format!("{t}{empty}")), expected);
    assert_eq!(
        ending(&format!("<div style='display:contents'>{t}</div>{empty}")),
        expected
    );
    assert_eq!(
        ending(&format!("{t}<div style='display:contents'>{empty}</div>")),
        expected
    );
    // An empty box of auto height between the two lets the margin through.
    assert_eq!(
        ending(&format!(
            "{t}<div style='display:contents'><div></div></div>{empty}"
        )),
        expected
    );
}
