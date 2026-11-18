//! A box whose `max-height` holds it below its last child: the child's
//! bottom margin no longer adjoins the box's bottom edge, so it neither
//! collapses through the box nor moves what follows, and the child that
//! overflows the box does not fit at the page end. The positions are the
//! ones Chromium prints for the same documents.

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

/// The 35px box `T` with a -50px bottom margin, in a box held to `max-height`
/// with a 10px bottom margin.
fn capped(lead: &str, max_height: &str) -> Vec<(u32, String, f32)> {
    runs(&format!(
        "<div style='width:20px;padding-bottom:15px'>{lead}\
         <div style='margin-bottom:10px;max-height:{max_height}'>\
         <div style='height:35px;margin-bottom:-50px'>T</div></div></div><div>Z</div>"
    ))
}

fn run(page: u32, text: &str, top: f32) -> (u32, String, f32) {
    (page, text.to_owned(), top)
}

#[test]
fn a_child_taller_than_the_maximum_keeps_its_margin_inside() {
    // 20px of box, its own 10px margin and the 15px padding below it.
    assert_eq!(capped("", "20px"), [run(0, "T", 5.0), run(0, "Z", 50.0)]);
    assert_eq!(capped("", "30px"), [run(0, "T", 5.0), run(0, "Z", 60.0)]);
}

#[test]
fn a_child_within_the_maximum_still_collapses_its_margin_through() {
    // 35px of box, then the margins 10px and -50px collapse into -40px; the
    // parent's 15px padding is all that is left.
    assert_eq!(capped("", "40px"), [run(0, "T", 5.0), run(0, "Z", 20.0)]);
}

#[test]
fn a_child_overflowing_its_capped_box_moves_to_the_next_page() {
    // The four lines of `X` fill the first two pages up to 60px. The 35px
    // child fits below them, but the capped box's 10px margin and the 15px
    // padding that close the wrapper after it do not, so the child moves to
    // the next page with them.
    let lead = "<div>X X X X</div>";
    let x = [
        run(0, "X", 5.0),
        run(0, "X", 35.0),
        run(1, "X", 5.0),
        run(1, "X", 35.0),
    ];
    let expect = |z: f32| {
        let mut runs = x.to_vec();
        runs.extend([run(2, "T", 5.0), run(2, "Z", z)]);
        runs
    };
    assert_eq!(capped(lead, "20px"), expect(50.0));
    assert_eq!(capped(lead, "30px"), expect(60.0));
}
