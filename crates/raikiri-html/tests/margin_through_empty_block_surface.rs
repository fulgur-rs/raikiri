//! The margins on both sides of an empty block collapse through it into one
//! margin, which ends where the block after it starts. A page edge inside that
//! margin cuts it, and the block after the empty one starts the next page, even
//! when a negative margin on it pulls its top above the empty block. The
//! positions are the ones Chromium prints for the same documents (`page.pdf`
//! with the page size taken from `@page`, the text read from the rendered
//! pages).

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

/// Ahem's ascent is 0.8em, which puts the top of a 20px glyph 16px above the
/// baseline.
const ASCENT: f32 = 16.0;

/// A block that fills the first page up to 10px short of its end, and ends in
/// a margin of `margin` that the page edge cuts.
fn lead_in(margin: i32) -> String {
    format!("<div style='height:90px;margin-bottom:{margin}px'>a</div>")
}

/// A block of one line, 30px tall, after a margin of `margin`.
fn block(margin: i32) -> String {
    format!("<div style='height:30px;margin-top:{margin}px'>X</div>")
}

/// The number of pages, and the page, left edge and top of every line of text
/// but the lead-in `a`. A 20px glyph in a 30px line sits 5px below the line.
fn pages(body: &str) -> (usize, Vec<(u32, f32, f32)>) {
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
    let mut count = 0;
    let mut lines = Vec::new();
    for index in 0..6 {
        let Some(page) = result.page(index) else {
            continue;
        };
        count += 1;
        for run in page.text_runs() {
            if run.text.trim_end() != "a" {
                lines.push((index, run.origin.0, run.origin.1 - ASCENT));
            }
        }
    }
    (count, lines)
}

/// The text of the block after the lead-in at the top of the second page.
fn at_top_of_second_page() -> (usize, Vec<(u32, f32, f32)>) {
    (2, vec![(1, 0.0, 5.0)])
}

#[test]
fn a_block_pulled_above_an_empty_block_starts_the_page_the_edge_opens() {
    // The margin of 30px and the one of -10px collapse into 20px that end at
    // 110, so the page edge at 100 cuts them and the block starts the page.
    let empty = "<div style='height:0'></div>";
    let body = format!("{}{empty}{}", lead_in(30), block(-10));
    assert_eq!(pages(&body), at_top_of_second_page());
    // The same margins without the empty block between.
    let body = format!("{}{}", lead_in(30), block(-10));
    assert_eq!(pages(&body), at_top_of_second_page());
}

#[test]
fn the_empty_block_does_not_move_the_flow_further_than_the_page_edge_does() {
    // The margins collapse into one that ends at 95, 80 or 85 or less: the
    // page edge at 100 lies outside it, the block starts above the edge, and
    // its line moves to the next page by itself.
    let empty = "<div style='height:0'></div>";
    for (margin, pulled) in [(30, -25), (30, -40), (40, -35)] {
        let body = format!("{}{empty}{}", lead_in(margin), block(pulled));
        assert_eq!(pages(&body), at_top_of_second_page(), "{margin} {pulled}");
    }
}

#[test]
fn a_margin_on_the_empty_block_joins_the_run() {
    let cases = [
        // A negative bottom margin on the empty block, and a positive and a
        // negative top margin on it.
        ("margin-bottom:-10px", 0),
        ("margin-top:20px", -10),
        ("margin-top:-10px", 0),
    ];
    for (style, pulled) in cases {
        let empty = format!("<div style='height:0;{style}'></div>");
        let body = format!("{}{empty}{}", lead_in(30), block(pulled));
        assert_eq!(pages(&body), at_top_of_second_page(), "{style}");
    }
}

#[test]
fn margins_that_stay_positive_are_cut_as_before() {
    let empty = "<div style='height:0'></div>";
    for (margin, next) in [(30, 10), (10, 30)] {
        let body = format!("{}{empty}{}", lead_in(margin), block(next));
        assert_eq!(pages(&body), at_top_of_second_page(), "{margin} {next}");
    }
}

#[test]
fn every_empty_block_before_the_block_belongs_to_the_run() {
    let empty = "<div style='height:0'></div>";
    let body = format!("{}{empty}{empty}{}", lead_in(30), block(-10));
    assert_eq!(pages(&body), at_top_of_second_page());
    // An empty block with an empty block inside it.
    let nested = "<div><div style='height:0'></div></div>";
    let body = format!("{}{nested}{}", lead_in(30), block(-10));
    assert_eq!(pages(&body), at_top_of_second_page());
}

#[test]
fn the_block_after_the_run_may_be_short_clipped_or_free_to_break() {
    let empty = "<div style='height:0'></div>";
    let cases = [
        "height:10px;margin-top:-10px",
        "height:4px;margin-top:-25px",
        "height:30px;margin-top:-10px;overflow:hidden",
        "height:30px;margin-top:-10px;break-before:avoid",
    ];
    for style in cases {
        let body = format!("{}{empty}<div style='{style}'>X</div>", lead_in(30));
        assert_eq!(pages(&body), at_top_of_second_page(), "{style}");
    }
}

#[test]
fn text_between_the_empty_block_and_the_block_ends_the_run() {
    // The text keeps the margins apart: the empty block and the text start
    // the second page, and the block follows the text, 10px closer than its
    // line would put it.
    let empty = "<div style='height:0'></div>";
    let body = format!("{}{empty}t{}", lead_in(30), block(-10));
    assert_eq!(pages(&body), (2, vec![(1, 0.0, 5.0), (1, 0.0, 25.0)]));
}

#[test]
fn an_out_of_flow_box_between_does_not_end_the_run() {
    let empty = "<div style='height:0'></div>";
    let out_of_flow = "<div style='position:absolute;width:0;height:0'></div>";
    let body = format!("{}{empty}{out_of_flow}{}", lead_in(30), block(-10));
    assert_eq!(pages(&body), at_top_of_second_page());
}

#[test]
fn the_run_is_cut_at_a_later_page_edge_too() {
    // The first block fills two pages and ends in the margin that the third
    // page's edge cuts.
    let lead = "<div style='height:190px;margin-bottom:130px'>a</div>";
    let empty = "<div style='height:0'></div>";
    let body = format!("{lead}{empty}{}", block(-10));
    assert_eq!(pages(&body), (3, vec![(2, 0.0, 5.0)]));
}

#[test]
fn a_forced_break_after_the_empty_block_keeps_the_margin_of_the_block_after_it() {
    // The empty block starts the second page, since the margin below the
    // lead-in crosses the page edge, and the break after it is forced: the
    // margin of the next block is kept in full on the third page. A negative
    // one pulls the block up over the page before it, where its text is drawn.
    let empty = "<div style='height:0;break-after:page'></div>";
    for (margin, page, top) in [
        (10, 2, 15.0),
        (-10, 2, -5.0),
        (-25, 1, 80.0),
        (-40, 1, 65.0),
    ] {
        let body = format!("{}{empty}{}", lead_in(30), block(margin));
        assert_eq!(pages(&body).1, vec![(page, 0.0, top)], "{margin}");
    }
}

#[test]
fn a_forced_break_after_a_box_inside_the_empty_block_ends_the_run_too() {
    // The break comes after the block that holds the empty one, wherever it is
    // set.
    let inside = "<div><div style='height:0;break-after:page'></div></div>";
    let wrapper = "<div style='break-after:page'><div style='height:0'></div></div>";
    for empty in [inside, wrapper] {
        let body = format!("{}{empty}{}", lead_in(30), block(-40));
        assert_eq!(pages(&body).1, vec![(1, 0.0, 65.0)], "{empty}");
    }
}

#[test]
fn an_empty_block_that_ends_the_flow_is_cut_by_its_own_margin() {
    // Nothing follows it, so there is no block to end the run, and nothing to
    // show but the page count.
    let body = format!("{}<div style='height:0'></div>", lead_in(30));
    assert!(pages(&body).1.is_empty());
}
