//! A break forced after an item of a flex line or a grid row does not fall
//! between that item and the items beside it: it waits for what follows the
//! line or row. Where each glyph falls is what Chromium prints for the same
//! documents on pages 100px tall, with Ahem lines of 20px and a line height
//! of 30px.

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

/// The page, glyph and glyph top of every glyph of `body` printed wholly
/// inside its page.
fn glyph_tops(body: &str) -> Vec<(u32, char, i32)> {
    let fonts = FontCollectionBuilder::new()
        .font_bytes(
            "Ahem",
            include_bytes!("../../raikiri-dom/tests/data/text-autospace/Ahem.ttf").to_vec(),
        )
        .build()
        .unwrap();
    let resources = RenderResources::new().fonts(fonts);
    let html = format!(
        "<!doctype html><style>@page{{size:100px 100px;margin:0}} body{{margin:0;font:20px/30px Ahem;orphans:1;widows:1}}</style><body>{body}</body>"
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
    let mut tops = Vec::new();
    for index in 0..result.page_count() {
        let page = result.page(index).unwrap();
        for run in page.text_runs() {
            let top = (run.origin.1 - run.ascent).round() as i32;
            if !(0..=80).contains(&top) {
                continue;
            }
            for glyph in run.text.chars().filter(|glyph| !glyph.is_whitespace()) {
                tops.push((index, glyph, top));
            }
        }
    }
    tops.sort_unstable();
    tops.dedup();
    tops
}

/// A document body and the page, glyph and top of each glyph.
type Row = (&'static str, &'static [(u32, char, i32)]);

fn assert_rows(rows: &[Row]) {
    for &(body, expected) in rows {
        assert_eq!(glyph_tops(body), expected, "{body}");
    }
}

#[test]
fn a_break_after_an_item_waits_for_the_end_of_its_line() {
    // A break after the first or the last item of a row flex line, after a
    // box inside an item, and after an item of a grid row: the items of the
    // line or row stay side by side, and what follows starts the next page.
    assert_rows(&[
        (
            "<div style='display:flex'><div style='break-after:page;width:40px'>A</div><div style='width:40px;height:40px'>B</div></div><div>Z</div>",
            &[(0, 'A', 5), (0, 'B', 5), (1, 'Z', 5)],
        ),
        (
            "<div style='display:flex'><div style='break-after:page;width:40px'>A</div><div style='width:40px;height:40px'></div></div><div>Z</div>",
            &[(0, 'A', 5), (1, 'Z', 5)],
        ),
        (
            "<div style='display:flex'><div style='width:40px'>A</div><div style='break-after:page;width:40px'>B</div></div><div>Z</div>",
            &[(0, 'A', 5), (0, 'B', 5), (1, 'Z', 5)],
        ),
        (
            "<div style='display:flex'><div style='width:40px'><p style='margin:0;break-after:page'>A</p></div><div style='width:40px'>B</div></div><div>Z</div>",
            &[(0, 'A', 5), (0, 'B', 5), (1, 'Z', 5)],
        ),
        (
            "<div style='display:grid;grid-template-columns:40px 40px'><div style='break-after:page'>A</div><div>B</div></div><div>Z</div>",
            &[(0, 'A', 5), (0, 'B', 5), (1, 'Z', 5)],
        ),
    ]);
}

#[test]
fn the_line_of_a_break_after_an_item_reaches_past_nesting_spans_and_alignment() {
    // An item of a nested flex line keeps the items beside its container; a
    // grid item spanning two rows keeps the items that start in them; the
    // lines of a container that wraps in reverse stay together; and an item
    // pushed down its line by a margin stays on it. A relative offset moves
    // no item onto or off a line: the next line still starts a new page.
    assert_rows(&[
        (
            "<div style='display:flex'><div style='display:flex'><div style='width:20px'>X</div><div style='break-after:page;width:20px'>A</div></div><div style='width:20px'>B</div></div><div>Z</div>",
            &[(0, 'A', 5), (0, 'B', 5), (0, 'X', 5), (1, 'Z', 5)],
        ),
        (
            "<div style='display:grid;grid-template-columns:40px 40px'><div style='grid-row:1/3;break-after:page'>A</div><div>B</div><div>C</div><div>D</div></div><div>Z</div>",
            &[
                (0, 'A', 5),
                (0, 'B', 5),
                (0, 'C', 35),
                (1, 'D', 5),
                (1, 'Z', 35),
            ],
        ),
        (
            "<div style='display:flex;flex-wrap:wrap-reverse;width:100px'><div style='break-after:page;width:60px'>A</div><div style='width:60px'>B</div></div><div>Z</div>",
            &[(0, 'A', 35), (0, 'B', 5), (1, 'Z', 5)],
        ),
        (
            "<div style='display:flex'><div style='break-after:page;width:20px'>A</div><div style='width:20px;margin-top:50px'>B</div><div style='width:20px;height:90px'></div></div><div>Z</div>",
            &[(0, 'A', 5), (0, 'B', 55), (1, 'Z', 5)],
        ),
        (
            "<div style='display:flex;flex-wrap:wrap;width:100px'><div style='position:relative;top:10px;break-after:page;width:60px'>A</div><div style='width:60px'>B</div></div><div>Z</div>",
            &[(0, 'A', 15), (1, 'B', 5), (1, 'Z', 35)],
        ),
    ]);
}

#[test]
fn a_break_after_an_item_still_starts_the_next_line_or_row_on_a_new_page() {
    // An item on the next flex line, the next item of a column flex
    // container, and the next row of a one-column grid start the next page.
    // A break before an item beside another is not taken at all. An item
    // that a break before it already moved to a new page keeps the next line
    // off that page. Gaps and centring that leave the next line between the
    // items of the line do not join the lines, in either main-axis direction.
    assert_rows(&[
        (
            "<div style='display:flex;flex-wrap:wrap;width:100px'><div style='break-after:page;width:60px'>A</div><div style='width:60px'>B</div></div><div>Z</div>",
            &[(0, 'A', 5), (1, 'B', 5), (1, 'Z', 35)],
        ),
        (
            "<div style='display:flex;flex-direction:column'><div style='break-after:page'>A</div><div>B</div></div><div>Z</div>",
            &[(0, 'A', 5), (1, 'B', 5), (1, 'Z', 35)],
        ),
        (
            "<div style='display:grid'><div style='break-after:page'>A</div><div>B</div></div><div>Z</div>",
            &[(0, 'A', 5), (1, 'B', 5), (1, 'Z', 35)],
        ),
        (
            "<div style='display:flex'><div style='width:40px'>A</div><div style='break-before:page;width:40px'>B</div></div><div>Z</div>",
            &[(0, 'A', 5), (0, 'B', 5), (0, 'Z', 35)],
        ),
        (
            "<div style='display:flex;flex-wrap:wrap;width:100px'><div style='width:60px'>X</div><div style='break-before:page;break-after:page;width:60px'>A</div><div style='width:60px'>B</div></div><div>Z</div>",
            &[(0, 'X', 5), (1, 'A', 5), (2, 'B', 5), (2, 'Z', 35)],
        ),
        (
            "<div style='display:flex;flex-wrap:wrap;width:100px'><div style='width:60px'>X</div><div style='break-before:page;break-after:page;width:40px'>A</div><div style='width:40px'>B</div><div style='width:60px'>C</div></div><div>Z</div>",
            &[
                (0, 'A', 5),
                (0, 'X', 5),
                (1, 'B', 5),
                (1, 'C', 5),
                (1, 'Z', 35),
            ],
        ),
        (
            "<div style='display:flex;flex-wrap:wrap;width:100px;column-gap:60px;justify-content:center'><div style='width:20px'>A</div><div style='break-after:page;width:20px'>B</div><div style='width:40px'>C</div></div><div>Z</div>",
            &[(0, 'A', 5), (0, 'B', 5), (1, 'C', 5), (1, 'Z', 35)],
        ),
        (
            "<div style='display:flex;flex-wrap:wrap;flex-direction:row-reverse;width:100px;column-gap:60px;justify-content:center'><div style='width:20px'>A</div><div style='break-after:page;width:20px'>B</div><div style='width:40px'>C</div></div><div>Z</div>",
            &[(0, 'A', 5), (0, 'B', 5), (1, 'C', 5), (1, 'Z', 35)],
        ),
        (
            "<div dir=rtl style='display:flex;flex-wrap:wrap;width:100px;column-gap:60px;justify-content:center'><div style='width:20px'>A</div><div style='break-after:page;width:20px'>B</div><div style='width:40px'>C</div></div><div>Z</div>",
            &[(0, 'A', 5), (0, 'B', 5), (1, 'C', 5), (1, 'Z', 35)],
        ),
    ]);
}

#[test]
fn a_break_before_a_later_item_with_a_margin_moves_its_whole_line() {
    // The items of the line start their margin boxes at its top, which the
    // break moves to the top of the next page, whatever the margins of the
    // item it is before, a negative one too.
    assert_rows(&[
        (
            "<div style='display:flex;flex-wrap:wrap;width:100px'><div style='width:100px;height:30px'>a</div><div style='width:40px;height:30px'>Y</div><div style='break-before:page;margin-top:10px;width:40px;height:30px'>X</div></div><div>Z</div>",
            &[(0, 'a', 5), (1, 'X', 15), (1, 'Y', 5), (1, 'Z', 45)],
        ),
        (
            "<div style='display:flex;flex-wrap:wrap;width:100px'><div style='width:100px;height:30px'>a</div><div style='width:40px;height:30px;margin-top:5px'>Y</div><div style='break-before:page;margin-top:10px;width:40px;height:30px'>X</div></div><div>Z</div>",
            &[(0, 'a', 5), (1, 'X', 15), (1, 'Y', 10), (1, 'Z', 45)],
        ),
        (
            "<div style='display:flex;flex-wrap:wrap;width:100px'><div style='width:100px;height:30px'>a</div><div style='width:40px;height:30px;margin-top:5px'>Y</div><div style='break-before:page;margin-top:-10px;width:40px;height:30px'>X</div></div><div>Z</div>",
            &[(0, 'a', 5), (1, 'Y', 10), (1, 'Z', 40)],
        ),
    ]);
}

#[test]
fn a_break_in_an_aligned_line_moves_the_top_of_the_line_to_the_page_top() {
    // The items keep their places in the line, as alignment and their
    // margins set them, below the top of the line, which the break moves
    // to the top of the next page. The line holds the items laid out side by
    // side, whatever their block positions.
    assert_rows(&[
        (
            "<div style='display:flex;flex-wrap:wrap;width:100px;align-items:flex-end'><div style='width:100px;height:30px'>a</div><div style='break-before:page;width:40px;height:30px;margin-top:-10px'>X</div><div style='width:40px;height:20px;margin-top:5px'>Y</div></div><div>Z</div>",
            &[(0, 'a', 5), (1, 'X', 0), (1, 'Y', 10), (1, 'Z', 30)],
        ),
        (
            "<div style='display:flex;flex-wrap:wrap;width:100px;align-items:flex-end'><div style='width:100px;height:30px'>a</div><div style='width:40px;height:20px'>Y</div><div style='break-before:page;width:40px;height:30px;margin-top:5px'>X</div></div><div>Z</div>",
            &[(0, 'a', 5), (1, 'X', 10), (1, 'Y', 20), (1, 'Z', 40)],
        ),
        (
            "<div style='display:flex;flex-wrap:wrap;width:100px;align-items:baseline'><div style='width:100px;height:30px'>a</div><div style='break-before:page;width:40px;height:30px;margin-top:-10px'>X</div><div style='width:40px;height:20px;margin-top:5px'>Y</div></div><div>Z</div>",
            &[(0, 'a', 5), (1, 'X', 10), (1, 'Y', 10), (1, 'Z', 40)],
        ),
        (
            "<div style='height:30px'>a</div><div style='display:flex;flex-wrap:nowrap;width:100px;align-items:baseline'><div style='break-before:page;width:40px;height:30px;margin-top:-10px'>X</div><div style='width:40px;height:20px;margin-top:5px'>Y</div></div><div>Z</div>",
            &[(0, 'a', 5), (1, 'X', 10), (1, 'Y', 10), (1, 'Z', 40)],
        ),
    ]);
}

#[test]
fn a_break_before_an_item_measures_its_line_where_the_flow_has_put_it() {
    assert_rows(&[
        // An earlier break in the container has moved the flow on, and the
        // next one still moves the line of its item to the top of a page.
        (
            "<div style='display:flex;flex-wrap:wrap;width:100px'><div style='width:100px;height:30px'>a</div><div style='break-before:page;width:100px;height:30px'>X</div><div style='break-before:page;width:40px;height:30px'>Q</div><div style='width:40px;height:30px'>R</div></div><div>Z</div>",
            &[
                (0, 'a', 5),
                (1, 'X', 5),
                (2, 'Q', 5),
                (2, 'R', 5),
                (2, 'Z', 35),
            ],
        ),
        // A line that already starts the page keeps it, though alignment puts
        // the item below the top of the line.
        (
            "<div style='height:100px'>a</div><div style='display:flex;flex-wrap:wrap;width:100px;align-items:flex-end'><div style='break-before:page;width:40px;height:30px'>X</div><div style='width:40px;height:50px'>Y</div></div>",
            &[(0, 'a', 5), (1, 'X', 25), (1, 'Y', 5)],
        ),
        // A break before the container has moved it on.
        (
            "<div>a</div><div style='break-before:page;height:70px'>b</div><div style='display:flex;flex-wrap:wrap;width:100px;align-items:flex-end'><div style='break-before:page;width:40px;height:30px'>X</div><div style='width:40px;height:50px'>Y</div></div>",
            &[(0, 'a', 5), (1, 'b', 5), (2, 'X', 25), (2, 'Y', 5)],
        ),
    ]);
}

#[test]
fn a_break_after_a_line_moves_the_top_of_the_next_line_to_the_page_top() {
    // A break after an item falls before the next flex line, whose top goes
    // to the top of the next page. The items keep their places in the line,
    // as alignment and their margins set them, so a taller item beside the
    // first one stays on the page.
    assert_rows(&[
        (
            "<div style='display:flex;flex-wrap:wrap;width:100px;align-items:flex-end'><div style='break-after:page;width:100px;height:30px'>a</div><div style='width:40px;height:30px;margin-top:-10px'>X</div><div style='width:40px;height:20px;margin-top:5px'>Y</div></div><div>Z</div>",
            &[(0, 'a', 5), (1, 'X', 0), (1, 'Y', 10), (1, 'Z', 30)],
        ),
        (
            "<div style='display:flex;flex-wrap:wrap;width:100px;align-items:flex-end'><div style='break-after:page;width:100px;height:30px'>a</div><div style='width:40px;height:30px'>Q</div><div style='width:40px;height:50px'>R</div></div><div>Z</div>",
            &[(0, 'a', 5), (1, 'Q', 25), (1, 'R', 5), (1, 'Z', 55)],
        ),
        (
            "<div style='display:flex;flex-wrap:wrap;width:100px;align-items:baseline'><div style='break-after:page;width:100px;height:30px'>a</div><div style='width:40px;height:30px;margin-top:10px'>Q</div><div style='width:40px;height:30px'>R</div></div><div>Z</div>",
            &[(0, 'a', 5), (1, 'Q', 15), (1, 'R', 15), (1, 'Z', 45)],
        ),
        // The text may lie deeper inside the item, in a flex item of its own.
        (
            "<div style='display:flex;flex-wrap:wrap;width:100px;align-items:flex-end'><div style='break-after:page;width:100px;height:30px'>a</div><div style='width:40px;height:30px;display:flex'><div style='width:40px;height:20px'>Q</div></div><div style='width:40px;height:50px'>R</div></div><div>Z</div>",
            &[(0, 'a', 5), (1, 'Q', 25), (1, 'R', 5), (1, 'Z', 55)],
        ),
        (
            "<div style='display:flex;flex-wrap:wrap;width:100px;align-items:flex-end'><div style='break-after:page;width:100px;height:30px'>a</div><div style='width:40px;margin-top:-10px'><div style='height:30px;margin-top:5px'>Q</div></div><div style='width:40px;height:50px'>R</div></div><div>Z</div>",
            &[(0, 'a', 5), (1, 'Q', 25), (1, 'R', 5), (1, 'Z', 55)],
        ),
        // An offset of an inline box around the text only paints it lower.
        (
            "<div style='display:flex;flex-wrap:wrap;width:100px;align-items:flex-end'><div style='break-after:page;width:100px;height:30px'>a</div><div style='width:40px;height:30px'><span style='position:relative;top:10px'>Q</span></div><div style='width:40px;height:50px'>R</div></div><div>Z</div>",
            &[(0, 'a', 5), (1, 'Q', 35), (1, 'R', 5), (1, 'Z', 55)],
        ),
        // A break after a box inside the item falls inside the item.
        (
            "<div style='display:flex'><div style='width:100px'>a<div style='break-after:page'>b</div>c</div></div>",
            &[(0, 'a', 5), (0, 'b', 35), (1, 'c', 5)],
        ),
    ]);
}

#[test]
fn a_break_at_the_start_of_an_item_moves_its_line() {
    // A break before the first box inside a flex item, however deep, is a
    // break before the item, and the whole line goes on to the next page.
    assert_rows(&[
        (
            "<div>X</div><div style='display:flex'><div>A</div><div><div style='break-before:page'>B</div></div></div>",
            &[(0, 'X', 5), (1, 'A', 5), (1, 'B', 5)],
        ),
        (
            "<div>X</div><div style='display:flex'><div>A</div><div><div><div style='break-before:page'>B</div></div></div></div>",
            &[(0, 'X', 5), (1, 'A', 5), (1, 'B', 5)],
        ),
        (
            "<div>X</div><div style='display:flex'><div>A<br>D</div><div><div style='break-before:page'>B</div></div></div>",
            &[(0, 'X', 5), (1, 'A', 5), (1, 'B', 5), (1, 'D', 35)],
        ),
        // A line that already starts the page keeps it, though alignment
        // puts the item below the top of the line.
        (
            "<div style='display:flex;align-items:flex-end'><div style='height:100px'>A</div><div><div style='break-before:page'>B</div></div></div>",
            &[(0, 'A', 5), (0, 'B', 75)],
        ),
        // Only the line of the item moves.
        (
            "<div>X</div><div style='display:flex;flex-wrap:wrap;width:100px'><div style='width:60px'>A</div><div style='width:60px'>C</div><div style='width:30px'><div style='break-before:page'>B</div></div></div>",
            &[(0, 'A', 35), (0, 'X', 5), (1, 'B', 5), (1, 'C', 5)],
        ),
    ]);
}
