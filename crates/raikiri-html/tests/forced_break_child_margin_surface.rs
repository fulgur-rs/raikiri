//! The margin above a box that starts a page after a forced break, when it
//! comes from the first child that collapses through the box. The positions
//! are the ones Chromium prints for the same documents.

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

/// Ahem's ascent is 0.8em, which puts the top of a 20px glyph 16px above the
/// baseline.
const ASCENT: f32 = 16.0;

/// A forced break after a 30px block.
const BREAK: &str = "<div style='break-after:page;height:30px'>a</div>";
/// Two lines of two words.
const TEXT: &str = "XX XX XX XX";

/// The page, left edge and top of every line of text but the lead-in `a`.
fn lines(body: &str) -> Vec<(u32, f32, f32)> {
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
    let mut lines = Vec::new();
    for index in 0..4 {
        let Some(page) = result.page(index) else {
            continue;
        };
        for run in page.text_runs() {
            if run.text.trim_end() != "a" {
                lines.push((index, run.origin.0, run.origin.1 - ASCENT));
            }
        }
    }
    lines
}

/// The lines of `TEXT` in a box styled `outer` after the forced break, holding
/// a box styled `inner`.
fn wrapped(outer: &str, inner: &str) -> Vec<(u32, f32, f32)> {
    lines(&format!(
        "{BREAK}<div style='{outer}'><div style='{inner}'>{TEXT}</div></div>"
    ))
}

#[test]
fn the_margin_of_a_first_child_stays_above_the_box_after_a_forced_break() {
    // The 10px margin of the child collapses through its parent, so it sits
    // above the parent as the parent's own margin would: the same positions
    // as a margin on the parent.
    let own = lines(&format!("{BREAK}<div style='margin-top:10px'>{TEXT}</div>"));
    assert_eq!(own, [(1, 0.0, 15.0), (1, 0.0, 45.0)]);
    assert_eq!(wrapped("", "margin-top:10px"), own);
    // Through two levels.
    let deep = lines(&format!(
        "{BREAK}<div><div><div style='margin-top:10px'>{TEXT}</div></div></div>"
    ));
    assert_eq!(deep, own);
}

#[test]
fn the_largest_margin_of_the_box_and_its_first_child_stays_above_it() {
    let expected = [(1, 0.0, 15.0), (1, 0.0, 45.0)];
    assert_eq!(wrapped("margin-top:5px", "margin-top:10px"), expected);
    assert_eq!(wrapped("margin-top:10px", "margin-top:5px"), expected);
    // A negative margin is added to the largest positive one.
    assert_eq!(
        wrapped("margin-top:10px", "margin-top:-5px"),
        [(1, 0.0, 10.0), (1, 0.0, 40.0)]
    );
}

#[test]
fn a_margin_that_does_not_collapse_through_the_box_stays_inside_it() {
    // The padding keeps the child's margin below the box's top edge, and an
    // overflow other than visible makes the box contain it.
    assert_eq!(
        wrapped("padding-top:1px", "margin-top:10px"),
        [(1, 0.0, 16.0), (1, 0.0, 46.0)]
    );
    assert_eq!(
        wrapped("overflow:hidden", "margin-top:10px"),
        [(1, 0.0, 15.0), (1, 0.0, 45.0)]
    );
}

#[test]
fn the_margin_of_a_first_child_stays_above_a_box_with_its_own_break_before() {
    let wrapped = lines(&format!(
        "<div style='height:30px'>a</div>\
         <div style='break-before:page'><div style='margin-top:10px'>{TEXT}</div></div>"
    ));
    assert_eq!(wrapped, [(1, 0.0, 15.0), (1, 0.0, 45.0)]);
}

#[test]
fn an_unforced_break_still_cuts_the_margin_of_a_first_child() {
    let unforced = lines(&format!(
        "<div style='height:90px'>a</div>\
         <div><div style='margin-top:10px'>{TEXT}</div></div>"
    ));
    assert_eq!(unforced, [(1, 0.0, 5.0), (1, 0.0, 35.0)]);
}

#[test]
fn the_margin_of_a_first_child_stays_above_the_box_beside_a_float() {
    let beside = lines(&format!(
        "{BREAK}<div style='float:left;width:40px;height:50px'></div>\
         <div><div style='margin-top:10px'>{TEXT}</div></div>"
    ));
    assert_eq!(beside, [(1, 40.0, 15.0), (1, 40.0, 45.0), (1, 0.0, 75.0)]);
}

#[test]
fn a_sibling_after_the_box_follows_the_margin_of_its_first_child() {
    let next = lines(&format!(
        "{BREAK}<div><div style='margin-top:10px;height:20px'></div></div><div>XX XX</div>"
    ));
    assert_eq!(next, [(1, 0.0, 35.0)]);
}
