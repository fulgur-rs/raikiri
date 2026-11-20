//! A forced break after a box inside another box that forces a break after
//! itself. The inner break falls before the next box inside the outer one,
//! and the outer break still falls after the outer box, so the content after
//! it starts one more page. Positions follow Chromium.

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderError, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, LimitKind, PageDefaults, RenderLimits};

/// Ahem's ascent is 0.8em, which puts the top of a 20px glyph 16px above the
/// baseline.
const ASCENT: f32 = 16.0;

/// A 20px block that forces a break after itself.
const BROKEN: &str = "<div style='height:20px;break-after:page'>T</div>";

/// The page count, and the page and top of the first run of every line, in
/// the order the pages hold them, on 100px pages with 20px/30px Ahem. Runs
/// side by side are in the order of their text.
fn layout_lines(body: &str) -> (u32, Vec<(u32, String, f32)>) {
    layout_lines_in(20, body)
}

/// [`layout_lines`] in a container `width` pixels wide.
fn layout_lines_in(width: u32, body: &str) -> (u32, Vec<(u32, String, f32)>) {
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
         <body><div style='width:{width}px'>{body}</div></body>"
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
        let page = result.page(index).expect("page in range");
        for run in page.text_runs() {
            lines.push((index, run.text.trim_end().to_owned(), run.origin.1 - ASCENT));
        }
    }
    lines.sort_by(|a, b| (a.0, a.2, &a.1).partial_cmp(&(b.0, b.2, &b.1)).unwrap());
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

#[test]
fn a_float_after_the_last_box_takes_the_one_break_of_both_boxes() {
    // Only a float follows the inner box: its break is the outer box's, the
    // float takes it and the line after the outer box goes on below it.
    assert_eq!(
        lines(&format!(
            "{}<div style='break-after:page'>{BROKEN}\
             <div style='float:left;width:20px'>F</div></div>Z",
            spacer(40)
        )),
        at(&[(0, "T", 45.0), (1, "F", 5.0), (1, "Z", 35.0)])
    );
    // With a line after the float, the outer break still falls after it.
    assert_eq!(
        lines(&format!(
            "{}<div style='break-after:page'>{BROKEN}\
             <div style='float:left;width:20px'>F</div>C</div>Z",
            spacer(40)
        )),
        at(&[(0, "T", 45.0), (1, "F", 5.0), (1, "C", 35.0), (2, "Z", 5.0)])
    );
}

#[test]
fn a_break_inside_a_cell_leaves_the_other_cells_of_the_row_in_place() {
    // The break moves the rest of the first cell; the second cell keeps its
    // first page, and the table's own break falls after the table.
    let (_, lines) = layout_lines_in(
        40,
        &format!(
            "{}<table style='break-after:page;border-spacing:0'><tr>\
             <td style='padding:0;width:20px'><div style='break-after:page'>T</div>X</td>\
             <td style='padding:0;width:20px'>Y</td></tr></table>Z",
            spacer(40)
        ),
    );
    let pages: Vec<_> = lines
        .iter()
        .map(|(page, text, _)| (*page, text.as_str()))
        .collect();
    assert_eq!(pages, [(0, "T"), (0, "Y"), (1, "X"), (2, "Z")], "{lines:?}");
}

#[test]
fn generated_content_before_the_boxes_leaves_the_last_box_at_the_end() {
    let expected = (
        2,
        at(&[(0, "P", 5.0), (0, "T", 35.0), (1, "F", 5.0), (1, "Z", 35.0)]),
    );
    // Only `::before` content of the outer box, a block or inline one: the
    // float after the inner box still takes the one break of both boxes.
    for display in ["block", "inline"] {
        assert_eq!(
            layout_lines(&format!(
                "<style>.o::before{{content:'P';display:{display}}}</style>\
                 <div class=o style='break-after:page'><div style='break-after:page'>T</div>\
                 <div style='float:left'>F</div></div>Z"
            )),
            expected,
            "{display}"
        );
    }
    // The same with the `::before` content on a box between them.
    assert_eq!(
        layout_lines(
            "<style>.p::before{content:'P'}</style><div style='break-after:page'>\
             <div class=p><div style='height:20px;break-after:page'>T</div>\
             <div style='float:left'>F</div></div></div>Z"
        ),
        expected
    );
}

#[test]
fn generated_content_after_the_boxes_leaves_the_first_box_at_the_start() {
    // Only `::after` content of a clipping box: a break before its first box
    // is a break before the box itself.
    assert_eq!(
        layout_lines(
            "<style>.o::after{content:'Q'}</style>A<div class=o \
             style='overflow:hidden;height:60px'><div style='break-before:page'>T</div></div>Z"
        ),
        (
            2,
            at(&[(0, "A", 5.0), (1, "T", 5.0), (1, "Q", 35.0), (1, "Z", 65.0)])
        )
    );
}

#[test]
fn a_generated_line_after_a_broken_box_starts_the_next_page() {
    let after =
        |display: &str| format!("<style>.g::after{{content:'x';display:{display}}}</style>");
    let inline = after("inline");
    let expected = |z: f32| (2, at(&[(0, "T", 45.0), (1, "x", 5.0), (1, "Z", z)]));
    // The line of `::after` text below the box takes the break before it,
    // and the flow after the paragraph follows the line, past the padding
    // that closes the paragraph too.
    for (body, z) in [
        (format!("{inline}<div class=g>{BROKEN}</div>Z"), 35.0),
        (
            format!("{inline}<div class=g style='padding-bottom:10px'>{BROKEN}</div>Z"),
            45.0,
        ),
        (
            format!("{inline}<div class=g style='padding-bottom:10px'>{BROKEN}</div><div>Z</div>"),
            45.0,
        ),
        (
            format!("{}<div class=g>{BROKEN}</div>Z", after("block")),
            35.0,
        ),
        // The break of a box at the end of the block above the line.
        (
            format!("{inline}<div class=g><div>{BROKEN}</div></div>Z"),
            35.0,
        ),
        // The block-end margin of the box ends with the page.
        (
            format!(
                "{inline}<div class=g><div style='height:20px;break-after:page;\
                 margin-bottom:10px'>T</div></div>Z"
            ),
            35.0,
        ),
    ] {
        assert_eq!(
            layout_lines(&format!("{}{body}", spacer(40))),
            expected(z),
            "{body}"
        );
    }
    // The break of a box that runs on to the next page opens the page after
    // that one.
    assert_eq!(
        layout_lines(&format!(
            "{}{inline}<div class=g><div style='height:150px;break-after:page'>T</div></div>Z",
            spacer(40)
        )),
        (3, at(&[(0, "T", 45.0), (2, "x", 5.0), (2, "Z", 35.0)]))
    );
    // The breaks of nested boxes that end together are one break.
    assert_eq!(
        layout_lines(&format!(
            "{}{inline}<div class=g><div style='break-after:page'>{BROKEN}</div></div>Z",
            spacer(40)
        )),
        expected(35.0)
    );
    // The lines of the box are placed before the line below it takes the
    // break: they move on together, and the line opens the page after them.
    assert_eq!(
        layout_lines(&format!(
            "{}{inline}<div class=g><div style='break-after:page'>A B</div></div>Z",
            spacer(50)
        )),
        (
            3,
            at(&[(1, "A", 5.0), (1, "B", 35.0), (2, "x", 5.0), (2, "Z", 35.0)])
        )
    );
    // A box with text of its own around the broken box.
    assert_eq!(
        layout_lines(&format!(
            "{}{inline}<div class=g><div>text{BROKEN}</div></div>Z",
            spacer(40)
        )),
        (
            2,
            at(&[
                (0, "text", 45.0),
                (0, "T", 75.0),
                (1, "x", 5.0),
                (1, "Z", 35.0)
            ])
        )
    );
    // At the end of the flow, also in a paragraph that keeps its height.
    for style in ["", "height:30px"] {
        assert_eq!(
            layout_lines(&format!(
                "{}{inline}<div class=g style='{style}'>{BROKEN}</div>",
                spacer(40)
            )),
            (2, at(&[(0, "T", 45.0), (1, "x", 5.0)])),
            "{style}"
        );
    }
    // The box ends at the page end, and inline boxes with no content follow
    // it in the paragraph.
    for body in [
        format!(
            "{}{inline}<div class=g><div style='height:40px;break-after:page'>T</div></div>Z",
            spacer(60)
        ),
        format!(
            "{}{inline}<div class=g>{BROKEN}<b><i></i></b></div>Z",
            spacer(40)
        ),
    ] {
        let lines = lines(&body);
        assert_eq!(&lines[1..], at(&[(1, "x", 5.0), (1, "Z", 35.0)]), "{body}");
    }
    // A line of atomic inlines is placed as a box, which takes the break
    // itself.
    assert_eq!(
        layout_lines(&format!(
            "{}<div>{BROKEN}<span style='display:inline-block;width:20px;height:20px'>i</span>\
             </div>Z",
            spacer(40)
        )),
        (2, at(&[(0, "T", 45.0), (1, "i", 5.0), (1, "Z", 35.0)]))
    );
    // A break with no such line after it leaves the line of an earlier
    // paragraph alone.
    assert_eq!(
        lines(&format!(
            "{inline}<div class=g><div>U</div></div><div style='height:20px;break-after:page'>A</div>Z"
        )),
        at(&[(0, "U", 5.0), (0, "x", 35.0), (0, "A", 65.0), (1, "Z", 5.0)])
    );
    // ... also at the end of the flow, where the break falls before nothing.
    assert_eq!(
        layout_lines(&format!(
            "{inline}<div class=g><div>U</div></div><div style='height:20px;break-after:page'>A</div>"
        )),
        (1, at(&[(0, "U", 5.0), (0, "x", 35.0), (0, "A", 65.0)]))
    );
    // At the top of the first page.
    assert_eq!(
        lines(&format!("{inline}<div class=g>{BROKEN}</div>Z")),
        at(&[(0, "T", 5.0), (1, "x", 5.0), (1, "Z", 35.0)])
    );
    // The break after the paragraph still falls after the line.
    assert_eq!(
        layout_lines(&format!(
            "{}{inline}<div class=g style='break-after:page'>{BROKEN}</div>Z",
            spacer(40)
        )),
        (3, at(&[(0, "T", 45.0), (1, "x", 5.0), (2, "Z", 5.0)]))
    );
    // A paragraph that keeps its height holds the moved line as overflow,
    // and the flow after it starts the next page too.
    assert_eq!(
        layout_lines(&format!(
            "{}{inline}<div class=g style='height:30px'>{BROKEN}</div>Z",
            spacer(40)
        )),
        (2, at(&[(0, "T", 45.0), (1, "Z", 5.0), (1, "x", 5.0)]))
    );
}

#[test]
fn a_generated_line_moved_past_the_page_limit_fails_the_layout() {
    let fonts = FontCollectionBuilder::new()
        .font_bytes(
            "Ahem",
            include_bytes!("../../raikiri-dom/tests/data/text-autospace/Ahem.ttf").to_vec(),
        )
        .build()
        .unwrap();
    let resources = RenderResources::new().fonts(fonts);
    // The line needs a second page, before the flow after it and at the end
    // of the flow.
    for after in ["Z", ""] {
        let html = format!(
            "<!doctype html><style>@page{{size:100px 100px;margin:0}}\
             body{{margin:0;font:20px/30px Ahem}}.g::after{{content:'x'}}</style>\
             <body><div style='width:20px'><div class=g>{BROKEN}</div>{after}</div></body>"
        );
        let document = parse_html_with_resources(html.as_bytes(), &resources).unwrap();
        let layout_with = |pages: u32| {
            layout(
                &document,
                PageDefaults::default(),
                LayoutConfig::builder()
                    .limits(
                        RenderLimits::builder()
                            .max_document_pages(Some(pages))
                            .build(),
                    )
                    .build(),
                LayoutOptions::new().resources(&resources),
            )
        };
        assert!(
            matches!(
                layout_with(1),
                Err(RenderError::LimitExceeded {
                    kind: LimitKind::Pages,
                    limit: 1,
                    ..
                })
            ),
            "{after}"
        );
        assert!(
            matches!(layout_with(2), Ok(LayoutStatus::Completed(_))),
            "{after}"
        );
    }
}
