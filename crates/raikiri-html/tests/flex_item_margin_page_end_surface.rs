//! Flex containers whose items end on the page while what follows them does
//! not. Chromium does not truncate a flex item's block-end margin at a page
//! break as block flow does: the page breaks before the container instead.
//! Where each glyph falls is what Chromium prints for the same documents on
//! pages 100px square, with Ahem glyphs of 20px on lines of 30px.

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

/// The page, glyph and glyph top of every line of text in `body`.
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
        "<!doctype html><style>@page{{size:100px 100px;margin:0}} body{{margin:0;font:20px/30px Ahem}}</style><body>{body}</body>"
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
            for glyph in run.text.chars().filter(|glyph| !glyph.is_whitespace()) {
                tops.push((index, glyph, (run.origin.1 - run.ascent).round() as i32));
            }
        }
    }
    tops.sort_unstable();
    tops.dedup();
    tops
}

/// A document and the page, glyph and top of each glyph Chromium shows of it.
type Row = (&'static str, &'static [(u32, char, i32)]);

/// Compares the glyphs that `expected` names; a glyph it leaves out is
/// one the Chromium print does not report in a way that can be compared.
fn assert_rows(rows: &[Row]) {
    for &(body, expected) in rows {
        let actual: Vec<_> = glyph_tops(body)
            .into_iter()
            .filter(|(_, glyph, _)| expected.iter().any(|(_, named, _)| named == glyph))
            .collect();
        assert_eq!(actual, expected, "{body}");
    }
}

/// An item that ends on the page but whose block-end margin runs past the page end
/// moves to the next page with its whole flex container: a row, a wrapping
/// row, and a column, also with more items before or after it, and nested in a
/// block or in another column (where only the inner container moves).
#[test]
fn an_item_margin_past_the_page_end_moves_the_container() {
    assert_rows(&[
        (
            "<div style='height:160px'>A</div><div style='display:flex'><div style='height:35px;margin-bottom:6px'>T</div></div><div>Z</div>",
            &[(0, 'A', 5), (2, 'T', 5), (2, 'Z', 46)],
        ),
        (
            "<div style='height:160px'>A</div><div style='display:flex'><div style='height:35px;margin-bottom:40px'>T</div></div><div>Z</div>",
            &[(0, 'A', 5), (2, 'T', 5), (3, 'Z', 5)],
        ),
        (
            "<div style='height:160px'>A</div><div style='display:flex;flex-direction:column'><div style='height:35px;margin-bottom:6px'>T</div></div><div>Z</div>",
            &[(0, 'A', 5), (2, 'T', 5), (2, 'Z', 46)],
        ),
        (
            "<div style='height:160px'>A</div><div style='display:flex;flex-wrap:wrap'><div style='height:35px;margin-bottom:6px'>T</div></div><div>Z</div>",
            &[(0, 'A', 5), (2, 'T', 5), (2, 'Z', 46)],
        ),
        (
            "<div style='height:160px'>A</div><div style='display:flex'><div style='height:35px;margin-bottom:6px'>T</div><div style='height:20px'>U</div></div><div>Z</div>",
            &[(0, 'A', 5), (2, 'T', 5), (2, 'U', 5), (2, 'Z', 46)],
        ),
        (
            "<div style='height:160px'>A</div><div style='display:flex;flex-direction:column'><div style='height:20px'>S</div><div style='height:15px;margin-bottom:6px'>T</div></div><div>Z</div>",
            &[(0, 'A', 5), (2, 'S', 5), (2, 'T', 25), (2, 'Z', 46)],
        ),
        (
            "<div style='height:150px'>A</div><div style='display:flex;flex-direction:column'><div style='height:30px'>S</div><div style='height:15px;margin-bottom:6px'>T</div><div>W</div></div><div>Z</div>",
            &[
                (0, 'A', 5),
                (2, 'S', 5),
                (2, 'T', 35),
                (2, 'W', 56),
                (3, 'Z', 5),
            ],
        ),
        (
            "<div style='height:130px'>A</div><div style='display:flex;flex-direction:column'><div style='height:30px'>S</div><div style='height:30px;margin-bottom:15px'>T</div><div style='height:30px'>W</div></div><div>Z</div>",
            &[
                (0, 'A', 5),
                (2, 'S', 5),
                (2, 'T', 35),
                (3, 'W', 5),
                (3, 'Z', 35),
            ],
        ),
        (
            "<div style='height:130px'>A</div><div style='display:flex;flex-wrap:wrap;width:100px'><div style='width:100px;height:30px'>S</div><div style='width:100px;height:30px;margin-bottom:15px'>T</div><div style='width:100px;height:30px'>W</div></div><div>Z</div>",
            &[
                (0, 'A', 5),
                (2, 'S', 5),
                (2, 'T', 35),
                (3, 'W', 5),
                (3, 'Z', 35),
            ],
        ),
        (
            "<div style='height:160px'>A</div><div style='display:flex'><div style='height:35px'>T</div><div style='height:30px;margin-bottom:11px'>U</div></div><div>Z</div>",
            &[(0, 'A', 5), (2, 'T', 5), (2, 'U', 5), (2, 'Z', 46)],
        ),
        (
            "<div style='height:40px'>A</div><div style='display:flex;flex-direction:column'><div style='height:30px'>S</div><div style='height:30px;margin-bottom:15px'>T</div></div><div>Z</div>",
            &[(0, 'A', 5), (1, 'S', 5), (1, 'T', 35), (2, 'Z', 5)],
        ),
        (
            "<div style='height:130px'>A</div><div style='display:flex;flex-direction:column'><div style='height:30px;margin-bottom:45px'>S</div><div style='height:30px'>T</div></div><div>Z</div>",
            &[(0, 'A', 5), (2, 'S', 5), (3, 'T', 5), (3, 'Z', 35)],
        ),
        (
            "<div style='height:130px'>A</div><div><div style='display:flex;flex-direction:column'><div style='height:30px'>S</div><div style='height:30px;margin-bottom:15px'>T</div></div></div><div>Z</div>",
            &[(0, 'A', 5), (2, 'S', 5), (2, 'T', 35), (3, 'Z', 5)],
        ),
        (
            "<div style='height:130px'>A</div><div style='padding-bottom:1px'><div style='display:flex'><div style='height:30px;margin-bottom:45px'>T</div></div></div><div>Z</div>",
            &[(0, 'A', 5), (2, 'T', 5), (3, 'Z', 5)],
        ),
        (
            "<div style='height:130px'>A</div><div style='height:30px'>B</div><div style='display:flex'><div style='height:30px;margin-bottom:15px'>T</div></div><div>Z</div>",
            &[(0, 'A', 5), (1, 'B', 35), (2, 'T', 5), (2, 'Z', 50)],
        ),
        (
            "<div style='height:160px'>A</div><div style='display:flex'><div style='height:30px;margin-bottom:15px'>T</div><div style='height:50px;margin-bottom:-20px'>U</div></div><div>Z</div>",
            &[(0, 'A', 5), (2, 'T', 5), (2, 'U', 5), (2, 'Z', 50)],
        ),
        (
            "<div style='height:130px'>A</div><div style='display:flex;flex-direction:column'><div style='height:30px'>S</div><div style='height:30px;margin-bottom:15px;break-inside:avoid'>T</div></div><div>Z</div>",
            &[(0, 'A', 5), (2, 'S', 5), (2, 'T', 35), (3, 'Z', 5)],
        ),
        (
            "<div style='height:130px'>A</div><div style='display:flex;flex-direction:column'><div style='height:30px'>S</div><div style='display:flex;flex-direction:column'><div style='height:30px;margin-bottom:15px'>T</div></div></div><div>Z</div>",
            &[(0, 'A', 5), (1, 'S', 35), (2, 'T', 5), (2, 'Z', 50)],
        ),
        (
            "<div style='height:160px'>A</div><div style='display:flex;height:41px'><div style='height:35px;margin-bottom:6px'>T</div></div><div>Z</div>",
            &[(0, 'A', 5), (2, 'T', 5), (2, 'Z', 46)],
        ),
    ]);
}

/// The padding that closes a row that does not wrap, or a column of one item,
/// moves the container with its single line; in a column of more items or a
/// wrapping row only the last item or line goes with it, as in block flow.
#[test]
fn closing_edges_move_a_single_line_container() {
    assert_rows(&[
        (
            "<div style='height:160px'>A</div><div style='display:flex;padding-bottom:6px'><div style='height:35px'>T</div></div><div>Z</div>",
            &[(0, 'A', 5), (2, 'T', 5), (2, 'Z', 46)],
        ),
        (
            "<div style='height:160px'>A</div><div style='display:flex;flex-direction:column;padding-bottom:6px'><div style='height:35px'>T</div></div><div>Z</div>",
            &[(0, 'A', 5), (2, 'T', 5), (2, 'Z', 46)],
        ),
        (
            "<div style='height:160px'>A</div><div style='display:flex;padding-bottom:15px'><div style='height:30px'>T</div><div style='height:30px'>U</div></div><div>Z</div>",
            &[(0, 'A', 5), (2, 'T', 5), (2, 'U', 5), (2, 'Z', 50)],
        ),
        (
            "<div style='height:160px'>A</div><div style='display:flex;padding-bottom:15px'>T</div><div>Z</div>",
            &[(0, 'A', 5), (2, 'T', 5), (2, 'Z', 50)],
        ),
        (
            "<div style='height:160px'>A</div><div style='display:flex;flex-direction:column;padding-bottom:15px'><div style='height:30px'>T</div></div><div>Z</div>",
            &[(0, 'A', 5), (2, 'T', 5), (2, 'Z', 50)],
        ),
        (
            "<div style='height:130px'>A</div><div style='display:flex;flex-direction:column;padding-bottom:15px'><div style='height:30px'>S</div><div style='height:30px'>T</div></div><div>Z</div>",
            &[(0, 'A', 5), (1, 'S', 35), (2, 'T', 5), (2, 'Z', 50)],
        ),
        (
            "<div style='height:130px'>A</div><div style='display:flex;flex-direction:column;padding-bottom:15px;border-bottom:1px solid'><div style='height:30px'>S</div><div style='height:30px'>T</div></div><div>Z</div>",
            &[(0, 'A', 5), (1, 'S', 35), (2, 'T', 5), (2, 'Z', 51)],
        ),
        (
            "<div style='height:130px'>A</div><div style='display:flex;flex-wrap:wrap;width:100px;padding-bottom:15px'><div style='width:100px;height:30px'>S</div><div style='width:100px;height:30px'>T</div></div><div>Z</div>",
            &[(0, 'A', 5), (1, 'S', 35), (2, 'T', 5), (2, 'Z', 50)],
        ),
        (
            "<div style='height:130px'>A</div><div><div style='display:flex;flex-direction:column;padding-bottom:15px'><div style='height:30px'>S</div><div style='height:30px'>T</div></div></div><div>Z</div>",
            &[(0, 'A', 5), (1, 'S', 35), (2, 'T', 5), (2, 'Z', 50)],
        ),
    ]);
}

/// A container stays where its items' margins fit, where it starts the page,
/// and where a margin reaches past its own end; a grid item's margin and a
/// block's last margin are truncated at the break as before. An item that
/// crosses the page end itself moves alone.
#[test]
fn containers_that_fit_or_start_the_page_stay() {
    assert_rows(&[
        (
            "<div style='height:160px'>A</div><div style='display:flex'><div style='height:35px;margin-bottom:4px'>T</div></div><div>Z</div>",
            &[(0, 'A', 5), (1, 'T', 65), (2, 'Z', 5)],
        ),
        (
            "<div style='height:160px'>A</div><div style='display:grid'><div style='height:35px;margin-bottom:6px'>T</div></div><div>Z</div>",
            &[(0, 'A', 5), (1, 'T', 65), (2, 'Z', 6)],
        ),
        (
            "<div style='height:160px'>A</div><div><div style='height:35px;margin-bottom:6px'>T</div></div><div>Z</div>",
            &[(0, 'A', 5), (1, 'T', 65), (2, 'Z', 5)],
        ),
        (
            "<div style='height:100px'>A</div><div style='display:flex;flex-direction:column'><div style='height:30px'>R</div><div style='height:30px'>S</div><div style='height:30px;margin-bottom:15px'>T</div></div><div>Z</div>",
            &[
                (0, 'A', 5),
                (1, 'R', 5),
                (1, 'S', 35),
                (1, 'T', 65),
                (2, 'Z', 10),
            ],
        ),
        (
            "<div style='display:flex'><div style='height:90px;margin-bottom:15px'>T</div></div><div>Z</div>",
            &[(0, 'T', 5), (1, 'Z', 10)],
        ),
        (
            "<div style='height:160px'>A</div><div style='display:flex;height:35px'><div style='height:20px;margin-top:15px;margin-bottom:6px'>T</div></div><div>Z</div>",
            &[(0, 'A', 5), (1, 'T', 80), (2, 'Z', 5)],
        ),
        (
            "<div style='height:160px'>A</div><div style='display:flex;margin-bottom:15px'><div style='height:30px'>T</div></div><div>Z</div>",
            &[(0, 'A', 5), (1, 'T', 65), (2, 'Z', 5)],
        ),
        (
            "<div style='height:160px'>A</div><div style='display:flex;flex-direction:column'><div style='height:20px'>S</div><div style='height:30px'>T</div></div><div>Z</div>",
            &[(0, 'A', 5), (1, 'S', 65), (2, 'T', 5), (2, 'Z', 35)],
        ),
        (
            "<div style='height:160px'>A</div><div style='display:flex;flex-direction:column'><div style='height:20px;margin-bottom:6px'>S</div><div style='height:30px'>T</div></div><div>Z</div>",
            &[(0, 'A', 5), (1, 'S', 65), (2, 'T', 5), (2, 'Z', 35)],
        ),
    ]);
}

/// The blocks a moving container begins go with it, keeping the padding and
/// border above it; an item inside a `display: contents` box counts, a break
/// forced after the item still follows on the next page, and a container
/// taller than a page, a float beside it or a table cell around it changes
/// nothing.
#[test]
fn containers_move_with_what_they_begin_and_past_what_surrounds_them() {
    assert_rows(&[
        (
            "<div style='height:130px'>A</div><div><div style='display:flex'><div style='display:contents'><div style='height:30px;margin-bottom:45px'>T</div></div></div></div><div>Z</div>",
            &[(0, 'A', 5), (2, 'T', 5), (3, 'Z', 5)],
        ),
        (
            "<div style='height:130px'>A</div><div style='display:flex;flex-direction:column'><div style='height:30px;margin-bottom:45px'>S</div><div style='height:30px;break-before:page'>T</div></div><div>Z</div>",
            &[(0, 'A', 5), (2, 'S', 5), (3, 'T', 5), (3, 'Z', 35)],
        ),
        (
            "<div style='height:160px'>A</div><div style='display:flex;height:500px'><div style='height:35px;margin-bottom:6px'>T</div></div><div>Z</div>",
            &[(0, 'A', 5), (2, 'T', 5), (7, 'Z', 5)],
        ),
        (
            "<div style='height:130px'>A</div><div style='display:flex;flex-direction:column'><div style='height:30px;margin-bottom:45px'>S</div><div style='height:30px'>T</div><div style='height:30px'>U</div><div style='height:30px'>V</div><div style='height:30px'>W</div></div><div>Z</div>",
            &[
                (0, 'A', 5),
                (2, 'S', 5),
                (3, 'T', 5),
                (3, 'U', 35),
                (3, 'V', 65),
                (4, 'W', 5),
                (4, 'Z', 35),
            ],
        ),
        (
            "<div style='height:130px'>A</div><div style='float:left;width:10px;height:100px'></div><div style='display:flex'><div style='height:30px;margin-bottom:45px'>T</div></div><div>Z</div>",
            &[(0, 'A', 5), (2, 'T', 5), (3, 'Z', 5)],
        ),
        (
            "<div style='height:130px'>A</div><table style='border-spacing:0'><tr><td style='padding:0'><div style='display:flex'><div style='height:30px;margin-bottom:45px'>T</div></div></td></tr></table><div>Z</div>",
            &[(0, 'A', 5), (2, 'T', 5), (3, 'Z', 5)],
        ),
        (
            "<div style='height:130px'>A</div><div style='padding-top:10px;border-top:1px solid'><div style='display:flex'><div style='height:30px;margin-bottom:45px'>T</div></div></div><div>Z</div>",
            &[(0, 'A', 5), (2, 'T', 16), (3, 'Z', 5)],
        ),
        (
            "<div style='height:130px'>A</div><div style='padding-top:10px'><div style='display:flex'><div style='height:30px;margin-bottom:45px'>T</div></div></div><div>Z</div>",
            &[(0, 'A', 5), (2, 'T', 15), (3, 'Z', 5)],
        ),
        // The item counts where it stands in the flow: it moves with the
        // container and is painted 20px above, across the page end. Chromium
        // also paints the part of its line below the page end on the next page.
        (
            "<div style='height:160px'>A</div><div style='display:flex'><div style='height:30px;margin-bottom:15px;position:relative;top:-20px'>T</div></div><div>Z</div>",
            &[(0, 'A', 5), (1, 'T', 85), (2, 'Z', 50)],
        ),
    ]);
}

/// A container inside a box that clips it at a set height, and one whose item
/// a relative offset paints past the page end while its place in the flow
/// fits, stay where they are.
#[test]
fn containers_stay_where_their_place_in_the_flow_fits_or_a_clip_holds_them() {
    assert_rows(&[
        (
            "<div style='height:130px'>A</div><div style='overflow:hidden;height:60px'><div style='display:flex'><div style='height:30px;margin-bottom:45px'>T</div></div></div><div>Z</div>",
            &[(0, 'A', 5), (1, 'T', 35), (2, 'Z', 5)],
        ),
        (
            "<div style='height:140px'>A</div><div style='display:flex'><div style='height:30px;margin-bottom:15px;position:relative;top:20px'>T</div></div><div>Z</div>",
            &[(0, 'A', 5), (1, 'T', 65), (2, 'Z', 5)],
        ),
    ]);
}
