//! A forced break before or after a block laid out inside an inline element
//! of a paragraph falls between that block and the lines around it, as it
//! does for a block that is a child of the paragraph's container (CSS
//! Fragmentation 3, section 3.1): the block keeps its place, and only what
//! follows the break moves to the next page.
//!
//! The pages are 100px square with no margin, and the text is 20px Ahem on
//! 30px lines, so a glyph's top sits 5px below its line's top.

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

const AHEM: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/data/text-autospace/Ahem.ttf"
));

/// The page, from 0, and the top on that page of the first fragment of each
/// element whose id is in `ids`.
fn place(body: &str, ids: &[&str]) -> Vec<(usize, f32)> {
    let fonts = FontCollectionBuilder::new()
        .font_bytes("Ahem", AHEM.to_vec())
        .build()
        .expect("fonts");
    let resources = RenderResources::new().fonts(fonts);
    let html = format!(
        "<!DOCTYPE html><style>@page {{ size: 100px 100px; margin: 0 }} \
         body {{ margin: 0; font: 20px/30px Ahem }}</style>\
         <div style='width: 20px'>{body}</div>"
    );
    let doc = parse_html_with_resources(html.as_bytes(), &resources).expect("parse");
    let status = layout(
        &doc,
        PageDefaults::default(),
        LayoutConfig::default(),
        LayoutOptions::new().resources(&resources),
    )
    .expect("layout");
    let LayoutStatus::Completed(result) = status else {
        panic!("expected a complete layout");
    };
    let mut found = vec![None; ids.len()];
    for (index, page) in result.pages().enumerate() {
        let dom = page.dom();
        for fragment in page.fragments() {
            let Some(id) = dom.attr(fragment.node(), "id") else {
                continue;
            };
            if let Some(slot) = ids.iter().position(|&wanted| wanted == id) {
                found[slot].get_or_insert((index, fragment.rect().y));
            }
        }
    }
    found
        .into_iter()
        .zip(ids)
        .map(|(place, id)| place.unwrap_or_else(|| panic!("no fragment for #{id}")))
        .collect()
}

fn assert_places(body: &str, expected: &[(&str, (usize, f32))]) {
    let ids: Vec<&str> = expected.iter().map(|(id, _)| *id).collect();
    let got = place(body, &ids);
    for ((id, want), got) in expected.iter().zip(got) {
        assert!(
            got.0 == want.0 && (got.1 - want.1).abs() <= 0.04,
            "#{id}: placed {got:?}, expected {want:?}"
        );
    }
}

/// The block that ends in the break stays on the first page, and the lines
/// after it start the next one; they do not take the block along and overlap
/// it there.
#[test]
fn a_break_after_a_block_in_a_span_moves_only_the_lines_after_it() {
    assert_places(
        "<div style='height: 40px'></div>\
         <div style='orphans: 2'><span>\
         <div style='height: 40px; break-after: page'><span id=t>T</span></div>\
         <span id=c>C</span><br><span id=d>C</span></span></div>\
         <div><span id=z>Z</span></div>",
        &[
            ("t", (0, 45.0)),
            ("c", (1, 5.0)),
            ("d", (1, 35.0)),
            ("z", (1, 65.0)),
        ],
    );
}

/// A block that runs on past the page end ends on the next page, so the
/// break after it opens the page after that one.
#[test]
fn a_break_after_a_block_in_a_span_that_crosses_a_page_skips_past_its_end() {
    assert_places(
        "<div style='height: 70px'></div>\
         <div><span>\
         <div style='height: 40px; break-after: page'><span id=t>T</span></div>\
         <span id=c>C</span></span></div>\
         <div><span id=z>Z</span></div>",
        &[("t", (0, 75.0)), ("c", (2, 5.0)), ("z", (2, 35.0))],
    );
}

/// A break before a block in a span leaves the line before it in place and
/// starts the next page with the block, the lines after it following.
#[test]
fn a_break_before_a_block_in_a_span_starts_the_next_page_with_it() {
    assert_places(
        "<div style='height: 30px'></div>\
         <div><span><span id=a>A</span>\
         <div style='height: 20px; break-before: page'><span id=t>T</span></div>\
         <span id=c>C</span></span></div>\
         <div><span id=z>Z</span></div>",
        &[
            ("a", (0, 35.0)),
            ("t", (1, 5.0)),
            ("c", (1, 25.0)),
            ("z", (1, 55.0)),
        ],
    );
    assert_places(
        "<div style='height: 40px'></div>\
         <div><span><span id=a>A</span>\
         <div style='height: 20px; break-before: page'><span id=t>T</span></div>\
         </span></div>\
         <div><span id=z>Z</span></div>",
        &[("a", (0, 45.0)), ("t", (1, 5.0)), ("z", (1, 25.0))],
    );
}

/// A break before a block that opens the paragraph inside a span, after an
/// empty box, moves the whole paragraph to the next page, and the lines after the block break
/// from there: the line that no longer fits takes the following content on
/// to the page after.
#[test]
fn a_break_before_a_block_that_opens_a_span_paginates_the_lines_after_it() {
    assert_places(
        "<div style='height: 0'></div>\
         <div><span>\
         <div style='height: 20px; break-before: page'><span id=t>T</span></div>\
         <span id=c>C</span><br><span id=d>C</span></span></div>\
         <div><span id=z>Z</span></div>",
        &[
            ("t", (1, 5.0)),
            ("c", (1, 25.0)),
            ("d", (1, 55.0)),
            ("z", (2, 5.0)),
        ],
    );
}

/// A block of another page type that ends in a break opens one page for the
/// lines after it, not one for the break and one for the page type.
#[test]
fn a_named_block_in_a_span_breaks_once_before_the_lines_after_it() {
    assert_places(
        "<style>@page a {}</style>\
         <div><span>\
         <div style='page: a; break-after: page; height: 40px'><span id=t>T</span></div>\
         <span id=c>C</span></span></div>",
        &[("t", (0, 5.0)), ("c", (1, 5.0))],
    );
    assert_places(
        "<style>@page a {}</style>\
         <div style='height: 20px'></div>\
         <div><span>\
         <div style='page: a; break-after: page; height: 40px'><span id=t>T</span></div>\
         <span id=c>C</span></span></div>",
        &[("t", (1, 5.0)), ("c", (2, 5.0))],
    );
}

/// A break before a block in a span that opens a table cell moves the whole
/// row, as it does for a block that is the cell's own first child. A span
/// with inline padding puts a line before the block, so the row stays and
/// only the block moves.
#[test]
fn a_break_before_a_block_in_a_span_at_a_cell_start_moves_the_row_unless_the_span_has_edges() {
    let row = |span: &str| {
        format!(
            "<div style='height: 30px'></div>\
             <table style='border-spacing: 0'><tr>\
             <td style='padding: 0'>{span}<div style='break-before: page'>\
             <span id=t>T</span></div></span></td>\
             <td style='padding: 0'><span id=x>X</span></td></tr></table>"
        )
    };
    assert_places(&row("<span>"), &[("t", (1, 5.0)), ("x", (1, 5.0))]);
    let edged = row("<span style='padding-left: 5px'>");
    let got = place(&edged, &["t", "x"]);
    assert_eq!(got[0], (1, 5.0), "#t");
    assert_eq!(got[1].0, 0, "#x stays on the first page: {:?}", got[1]);
}

/// The margin after a block that ends in a break is truncated at the break:
/// a margin that reaches past the page end does not send the lines after the
/// block one page further.
#[test]
fn a_break_after_a_block_in_a_span_drops_the_margin_after_it() {
    assert_places(
        "<div style='height: 60px'></div>\
         <div><span>\
         <div style='height: 20px; margin-bottom: 40px; break-after: page'>\
         <span id=t>T</span></div>\
         <span id=c>C</span></span></div>",
        &[("t", (0, 65.0)), ("c", (1, 5.0))],
    );
}

/// A block that ends right at the page end ends on that page: the break after
/// it opens the next page, not the one after.
#[test]
fn a_break_after_a_block_in_a_span_that_ends_at_the_page_end_opens_the_next_page() {
    assert_places(
        "<div style='height: 60px'></div>\
         <div><span>\
         <div style='height: 40px; break-after: page'><span id=t>T</span></div>\
         <span id=c>C</span></span></div>",
        &[("t", (0, 65.0)), ("c", (1, 5.0))],
    );
}

/// A line break before the block in a span is content of a line of its own,
/// so a break before the block leaves the row in place and moves only the
/// block.
#[test]
fn a_break_before_a_block_after_a_line_break_in_a_span_at_a_cell_start_keeps_the_row() {
    assert_places(
        "<div style='height: 30px'></div>\
         <table style='border-spacing: 0'><tr>\
         <td style='padding: 0'><span><br></span><div style='break-before: page'>\
         <span id=t>T</span></div></td>\
         <td style='padding: 0'><span id=x>X</span></td></tr></table>",
        &[("t", (1, 5.0)), ("x", (0, 55.0))],
    );
}

/// A relative offset only paints the block lower: the break after it follows
/// where the block ends in the flow, so the lines after it start the next
/// page, not the one after.
#[test]
fn a_break_after_a_relatively_offset_block_in_a_span_follows_its_flow_place() {
    assert_places(
        "<div style='height: 60px'></div>\
         <div><span>\
         <div style='height: 20px; position: relative; top: 40px; break-after: page'>\
         <span id=t>T</span></div>\
         <span id=c>C</span></span></div>",
        &[("c", (1, 5.0))],
    );
}

/// The paragraph around the block and the edges below it, given as the
/// block's wrapper inside the paragraph, the content after the block, and the
/// paragraph's style, below a spacer `above` px tall.
fn edges_case(above: u32, wrapper: &str, after: &str, style: &str) -> String {
    let block = "<div style='height: 20px; break-after: page'><span id=t>T</span></div>";
    let inner = match wrapper {
        "span" => format!("<span>{block}</span>"),
        "spans" => format!("<span><span>{block}</span></span>"),
        _ => block.to_string(),
    };
    format!(
        "<div style='width: 20px'><div style='height: {above}px'></div>\
         <div style='{style}'>{inner}{after}</div>\
         <div><span id=z>Z</span></div></div>"
    )
}

/// A span around a block that ends in a break closes with a line of its own,
/// empty as it is, after the block: the break carries that line and the
/// paragraph's bottom padding or border to the next page, where they fit.
#[test]
fn a_break_after_a_block_in_a_span_takes_the_paragraph_edges_along() {
    assert_places(
        &edges_case(40, "span", "", "padding-bottom: 10px"),
        &[("t", (0, 45.0)), ("z", (1, 15.0))],
    );
    assert_places(
        &edges_case(0, "spans", "", "border-bottom: 10px solid"),
        &[("t", (0, 5.0)), ("z", (1, 15.0))],
    );
    assert_places(
        &edges_case(0, "span", " ", "padding-bottom: 10px; margin-bottom: 15px"),
        &[("t", (0, 5.0)), ("z", (1, 30.0))],
    );
}

/// An inline box after a block of the paragraph makes the closing line too,
/// while white space that collapses away does not.
#[test]
fn only_inline_content_after_a_paragraph_block_takes_the_edges_along() {
    assert_places(
        &edges_case(40, "", "<span></span>", "padding-bottom: 10px"),
        &[("t", (0, 45.0)), ("z", (1, 15.0))],
    );
    assert_places(
        &edges_case(40, "", "", "padding-bottom: 10px"),
        &[("t", (0, 45.0)), ("z", (1, 5.0))],
    );
}

/// Content after a block of the paragraph that renders nothing in the line,
/// such as collapsed white space, a box that is not displayed, an
/// out-of-flow box or an empty `display: contents` box, leaves no closing
/// line: the edges stay below the paragraph. A box inside a `display:
/// contents` box makes that line.
#[test]
fn a_paragraph_block_ends_with_a_line_only_for_inline_content_after_it() {
    for (after, z) in [
        (" ", (1, 5.0)),
        ("<span style='display: none'></span>", (1, 5.0)),
        ("<span style='position: absolute'></span>", (1, 5.0)),
        ("<b style='display: contents'></b>", (1, 5.0)),
        ("<b style='display: contents'> </b>", (1, 5.0)),
        ("<b style='display: contents'><span></span></b>", (1, 15.0)),
    ] {
        assert_places(
            &edges_case(40, "", after, "padding-bottom: 10px"),
            &[("t", (0, 45.0)), ("z", z)],
        );
    }
}

/// A break inside an inline-block in a span stays inside the inline-block:
/// the paragraph's edges stay below the paragraph.
#[test]
fn a_break_inside_an_inline_block_in_a_span_leaves_the_edges() {
    let html = edges_case(40, "span", "", "padding-bottom: 10px").replace(
        "<span><div style='height: 20px; break-after: page'><span id=t>T</span></div></span>",
        "<span><span style='display: inline-block; width: 20px'>\
         <div style='height: 20px; break-after: page'><span id=t>T</span></div></span></span>",
    );
    assert!(html.contains("inline-block"));
    assert_places(&html, &[("t", (0, 45.0)), ("z", (1, 5.0))]);
}

/// Edges that do not fit below a block in a span that ends in a break start
/// the next page with the closing line, which takes the break: the box after
/// them follows on that page.
#[test]
fn edges_that_start_the_next_page_take_the_break_in_their_span() {
    assert_places(
        &edges_case(0, "span", "", "padding-bottom: 70px").replace("height: 20px", "height: 50px"),
        &[("t", (0, 5.0)), ("z", (1, 75.0))],
    );
}

/// Content that ends at the page end leaves the closing line and the edges
/// to start the next page, which takes the break.
#[test]
fn edges_below_a_block_in_a_span_that_ends_at_the_page_end_take_the_break() {
    assert_places(
        &edges_case(60, "span", "", "padding-bottom: 10px").replace("height: 20px", "height: 40px"),
        &[("t", (0, 65.0)), ("z", (1, 15.0))],
    );
}

/// A break inside a float in the span stays inside the float: no line of the
/// paragraph follows it, and the edges stay below the paragraph.
#[test]
fn a_break_inside_a_float_in_a_span_leaves_the_edges() {
    assert_places(
        "<div style='width: 20px'><div style='height: 40px'></div>\
         <div style='padding-bottom: 10px'><span><div style='float: left; width: 20px'>\
         <div style='height: 20px; break-after: page'><span id=t>T</span></div></div></span></div>\
         <div><span id=z>Z</span></div></div>",
        &[("t", (0, 45.0)), ("z", (0, 65.0))],
    );
}

/// A break forced after the paragraph, or after a box around it, still
/// falls after it once the line that closes the paragraph has taken the
/// break after a block in a span, and the paragraph's bottom edges with it:
/// the box after it starts a third page. Breaks after boxes that end before
/// that line go with the one it takes.
#[test]
fn a_break_after_the_paragraph_follows_the_closing_line_that_takes_the_inner_break() {
    let block = "<div style='height: 20px; break-after: page'><span id=t>T</span></div>";
    for (above, outer, inner, t) in [
        (
            "",
            "break-after: page; padding-bottom: 10px",
            format!("<span>{block}</span>"),
            (0, 5.0),
        ),
        (
            "",
            "break-after: page; padding-bottom: 10px",
            format!("<span><div style='break-after: page'>{block}</div></span>"),
            (0, 5.0),
        ),
        (
            "",
            "break-after: page",
            format!("<div style='padding-bottom: 10px'><span>{block}</span></div>"),
            (0, 5.0),
        ),
        (
            "<div style='height: 40px'></div>",
            "break-after: page; padding-bottom: 10px",
            format!("<span>{block}</span>"),
            (0, 45.0),
        ),
    ] {
        assert_places(
            &format!(
                "<div style='width: 20px'>{above}<div style='{outer}'>{inner}</div>\
                 <div><span id=z>Z</span></div></div>"
            ),
            &[("t", t), ("z", (2, 5.0))],
        );
    }
}
