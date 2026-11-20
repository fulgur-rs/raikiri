//! A forced break after a box inside another box that forces a break after
//! itself. The inner break falls before the next box inside the outer one,
//! and the outer break still falls after the outer box, so the content after
//! it starts one more page. Positions follow Chromium.

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

/// Ahem's ascent is 0.8em, which puts the top of a 20px glyph 16px above the
/// baseline.
const ASCENT: f32 = 16.0;

/// A 20px block that forces a break after itself.
const BROKEN: &str = "<div style='height:20px;break-after:page'>T</div>";

/// The page count, and the page and top of the first run of every line, in
/// the order the pages hold them, on 100px pages with 20px/30px Ahem.
fn layout_lines(body: &str) -> (u32, Vec<(u32, String, f32)>) {
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
         body{{margin:0;font:20px/30px Ahem}}</style>\
         <body><div style='width:20px'>{body}</div></body>"
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
    for index in 0..result.page_count() {
        let page = result.page(index as usize).expect("page in range");
        for run in page.text_runs() {
            lines.push((index, run.text.trim_end().to_owned(), run.origin.1 - ASCENT));
        }
    }
    lines.sort_by(|a, b| (a.0, a.2).partial_cmp(&(b.0, b.2)).unwrap());
    (result.page_count(), lines)
}

/// The page, text and top of every line, for comparison with `expected`.
fn lines(body: &str) -> Vec<(u32, String, f32)> {
    layout_lines(body).1
}

/// `expected` in the shape [`lines`] returns.
fn at(expected: &[(u32, &str, f32)]) -> Vec<(u32, String, f32)> {
    expected
        .iter()
        .map(|&(page, text, top)| (page, text.to_owned(), top))
        .collect()
}

fn spacer(height: u32) -> String {
    format!("<div style='height:{height}px'></div>")
}

#[test]
fn a_break_after_a_child_still_breaks_before_its_next_sibling() {
    let expected = at(&[(0, "T", 45.0), (1, "C", 5.0), (2, "Z", 5.0)]);
    // A block, or a line of text, after the child.
    for after in ["<div>C</div>", "C"] {
        assert_eq!(
            lines(&format!(
                "{}<div style='break-after:page'>{BROKEN}{after}</div>Z",
                spacer(40)
            )),
            expected,
            "{after}"
        );
    }
    // The outer box at the top of the first page.
    assert_eq!(
        lines(&format!(
            "<div style='break-after:page'>{BROKEN}<div>C</div></div>Z"
        )),
        at(&[(0, "T", 5.0), (1, "C", 5.0), (2, "Z", 5.0)])
    );
}

#[test]
fn a_break_after_a_box_deeper_inside_still_breaks_before_what_follows_it() {
    let expected = at(&[(0, "T", 45.0), (1, "C", 5.0), (2, "Z", 5.0)]);
    // Inside a wrapper, with the text after the box in the wrapper or in a
    // block of its own, and inside an inline box around the box.
    for inner in [
        format!("<div>{BROKEN}C</div>"),
        format!("<div>{BROKEN}<div>C</div></div>"),
        format!("<div><span>{BROKEN}C</span></div>"),
        format!("<span>{BROKEN}C</span>"),
    ] {
        assert_eq!(
            lines(&format!(
                "{}<div style='break-after:page'>{inner}</div>Z",
                spacer(40)
            )),
            expected,
            "{inner}"
        );
    }
    // A box between the two that forces a break after itself too.
    assert_eq!(
        lines(&format!(
            "{}<div style='break-after:page'><div style='break-after:page'>\
             {BROKEN}<div>C</div></div><div>D</div></div>Z",
            spacer(40)
        )),
        at(&[(0, "T", 45.0), (1, "C", 5.0), (2, "D", 5.0), (3, "Z", 5.0)])
    );
}

#[test]
fn the_content_after_the_inner_break_keeps_the_outer_break_after_it() {
    // A line after the wrapper of the box, inside the outer box.
    assert_eq!(
        lines(&format!(
            "{}<div style='break-after:page'><div>{BROKEN}C</div>D</div>Z",
            spacer(40)
        )),
        at(&[(0, "T", 45.0), (1, "C", 5.0), (1, "D", 35.0), (2, "Z", 5.0)])
    );
    // Two breaks in a row inside the outer box.
    assert_eq!(
        lines(&format!(
            "{}<div style='break-after:page'>{BROKEN}\
             <div style='height:20px;break-after:page'>C</div><div>D</div></div>Z",
            spacer(40)
        )),
        at(&[(0, "T", 45.0), (1, "C", 5.0), (2, "D", 5.0), (3, "Z", 5.0)])
    );
    // The outer box followed by another that forces a break after itself.
    assert_eq!(
        lines(&format!(
            "{}<div style='break-after:page'>{BROKEN}C</div>\
             <div style='break-after:page'>D</div>Z",
            spacer(40)
        )),
        at(&[(0, "T", 45.0), (1, "C", 5.0), (2, "D", 5.0), (3, "Z", 5.0)])
    );
}

#[test]
fn breaks_after_the_last_box_and_after_the_outer_box_are_one_break() {
    // The inner break falls where the outer box ends: one break, whether
    // the box is the outer box's child or inside a padded wrapper, and the
    // last of two broken boxes adds no page to the first one's break.
    for inner in [
        BROKEN.to_owned(),
        format!("<div style='padding-bottom:10px'>{BROKEN}</div>"),
        format!("{BROKEN}<div style='height:20px;break-after:page'>C</div>"),
    ] {
        let (pages, lines) = layout_lines(&format!(
            "{}<div style='break-after:page'>{inner}</div>Z",
            spacer(40)
        ));
        let z = lines.iter().find(|line| line.1 == "Z").unwrap();
        let expected_page = if inner.contains(">C<") { 2 } else { 1 };
        assert_eq!((z.0, z.2), (expected_page, 5.0), "{inner}");
        assert_eq!(pages, expected_page + 1, "{inner}");
    }
}

#[test]
fn a_break_after_a_flex_item_or_a_cell_box_keeps_the_outer_break() {
    // A column flex container and a grid stack their items like blocks, and
    // a forced break after the first item falls before the second.
    for display in ["flex;flex-direction:column", "grid"] {
        assert_eq!(
            lines(&format!(
                "{}<div style='break-after:page;display:{display}'>{BROKEN}<div>C</div></div>Z",
                spacer(40)
            )),
            at(&[(0, "T", 45.0), (1, "C", 5.0), (2, "Z", 5.0)]),
            "{display}"
        );
    }
    // The same inside a table cell.
    assert_eq!(
        lines(&format!(
            "{}<table style='break-after:page;border-spacing:0'><tr><td style='padding:0'>\
             {BROKEN}<div>C</div></td></tr></table>Z",
            spacer(40)
        )),
        at(&[(0, "T", 45.0), (1, "C", 5.0), (2, "Z", 5.0)])
    );
}
