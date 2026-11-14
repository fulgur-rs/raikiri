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
