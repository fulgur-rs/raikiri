//! Where a box lands when the block-start margin that a forced page break
//! keeps above it runs past the end of the page the break opens. Chromium
//! cuts the margin at that page end, as it cuts one at an unforced break, and
//! the box starts the next page. The positions are the ones Chromium prints
//! for the same documents.

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

/// Ahem's ascent is 0.8em, which puts the top of a 10px glyph 8px above the
/// baseline.
const ASCENT: f32 = 8.0;
/// The advance of one Ahem glyph at 10px.
const ADVANCE: f32 = 10.0;

/// Every word of `body` as `word@page:x,y`, the left and top of its glyphs on
/// its page, on 100x100 pages with no margin and 10px Ahem lines 10px apart.
fn words(body: &str) -> String {
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
    let mut words = Vec::new();
    for index in 0..result.page_count() {
        let page = result.page(index).expect("a page");
        for run in page.text_runs() {
            let mut x = run.origin.0;
            let y = (run.origin.1 - ASCENT).round();
            for word in run.text.split(' ') {
                if !word.trim().is_empty() {
                    words.push(format!("{}@{index}:{},{}", word.trim(), x.round(), y));
                }
                x += ADVANCE * (word.chars().count() + 1) as f32;
            }
        }
    }
    words.join(" ")
}

/// A line, then a box with `style` holding `content`, after a forced break,
/// then a line.
fn after_forced_break(style: &str, content: &str) -> String {
    words(&format!(
        "<div>A</div><div style='break-before:page;{style}'>{content}</div><div>X</div>"
    ))
}

#[test]
fn a_kept_margin_past_the_page_end_is_cut_there() {
    for margin in [100, 150, 250, 350] {
        assert_eq!(
            after_forced_break(&format!("margin-top:{margin}px"), "S"),
            "A@0:0,0 S@2:0,0 X@2:0,10",
            "{margin}"
        );
    }
    assert_eq!(
        after_forced_break("margin-top:150px;height:30px", "S"),
        "A@0:0,0 S@2:0,0 X@2:0,30"
    );
    assert_eq!(
        after_forced_break("margin-top:150px;padding-top:5px", "S"),
        "A@0:0,0 S@2:0,5 X@2:0,15"
    );
    assert_eq!(
        after_forced_break("margin-top:150px;width:10px", "S T U V W"),
        "A@0:0,0 S@2:0,0 T@2:0,10 U@2:0,20 V@2:0,30 W@2:0,40 X@2:0,50"
    );
    for display in ["flow-root", "flex"] {
        assert_eq!(
            after_forced_break(&format!("margin-top:150px;display:{display}"), "S"),
            "A@0:0,0 S@2:0,0 X@2:0,10",
            "{display}"
        );
    }
}

#[test]
fn a_kept_margin_that_fits_its_page_stays() {
    assert_eq!(
        after_forced_break("margin-top:60px;height:60px", "S"),
        "A@0:0,0 S@1:0,60 X@2:0,20"
    );
    assert_eq!(
        after_forced_break("margin-top:-5px", "S"),
        "A@0:0,0 S@1:0,-5 X@1:0,5"
    );
}

#[test]
fn a_margin_kept_through_other_boxes_is_cut_too() {
    // The margin of a child collapses through the top of the box that breaks.
    assert_eq!(
        after_forced_break("margin-top:150px", "<div style='margin-top:20px'>S</div>"),
        "A@0:0,0 S@2:0,0 X@2:0,10"
    );
    assert_eq!(
        words(
            "<div>A</div><div style='break-before:page'><div style='margin-top:150px'>S</div></div><div>X</div>"
        ),
        "A@0:0,0 S@2:0,0 X@2:0,10"
    );
    // A break after an empty box.
    assert_eq!(
        words(
            "<div>A</div><div style='break-after:page'></div><div style='margin-top:150px'>S</div><div>X</div>"
        ),
        "A@0:0,0 S@2:0,0 X@2:0,10"
    );
    // A break at the end of a full page.
    assert_eq!(
        words(
            "<div style='height:100px'>A</div><div style='break-before:page;margin-top:150px'>S</div><div>X</div>"
        ),
        "A@0:0,0 S@2:0,0 X@2:0,10"
    );
    // An empty box, whose margins collapse through it with the next box's.
    assert_eq!(
        words(
            "<div>A</div><div style='break-before:page;margin-top:150px'></div><div style='margin-top:20px'>X</div>"
        ),
        "A@0:0,0 X@2:0,0"
    );
    // A page-type change breaks the page as a forced break does.
    for (margin, page) in [(50, "S@1:0,50"), (150, "S@2:0,0"), (250, "S@2:0,0")] {
        let x_page = if margin == 50 { 2 } else { 3 };
        assert_eq!(
            words(&format!(
                "<style>@page x{{size:100px 100px;margin:0}}</style><div>A</div>\
                 <div style='page:x;margin-top:{margin}px'>S</div><div>X</div>"
            )),
            format!("A@0:0,0 {page} X@{x_page}:0,0"),
            "{margin}"
        );
    }
    // A table.
    assert_eq!(
        words(
            "<div>A</div><table style='break-before:page;margin-top:150px;border-spacing:0'><tr><td style='padding:0'>S</td></tr></table><div>X</div>"
        ),
        "A@0:0,0 S@2:0,0 X@2:0,10"
    );
}

#[test]
fn a_spanner_cuts_its_kept_margin_as_a_block_does() {
    for (before, margin) in [("", 150), ("", 250), ("height:100px", 150)] {
        assert_eq!(
            words(&format!(
                "<div style='columns:2;column-gap:0'><div style='{before}'>A</div>\
                 <div style='column-span:all;break-before:page;margin-top:{margin}px'>S</div>\
                 <div>X</div></div>"
            )),
            "A@0:0,0 S@2:0,0 X@2:0,10",
            "{before} {margin}"
        );
    }
    assert_eq!(
        words(
            "<div style='columns:2;column-gap:0'><div>A</div>\
             <div style='column-span:all;break-before:page;margin-top:50px'>S</div>\
             <div>X</div></div>"
        ),
        "A@0:0,0 S@1:0,50 X@1:0,60"
    );
}
