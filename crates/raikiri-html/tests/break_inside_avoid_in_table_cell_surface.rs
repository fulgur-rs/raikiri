//! A block that avoids a break inside it (`break-inside: avoid`) in the flow
//! of a table cell. As in any block flow (CSS Fragmentation 3 §3.2), a block
//! that would cross the page end moves to the next page whole; the content
//! after it in the cell follows, the row grows to hold it, and the flow after
//! the table follows the row's end. The cells beside it keep their places.
//! Where each glyph falls is what Chromium prints for the same documents on
//! pages 200px wide and 100px tall, with Ahem glyphs of 20px on lines of 20px.

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

/// The page, glyph and glyph top of every word of text in `body`, laid out
/// on the pages and with the font of the Chromium documents.
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
        "<!doctype html><style>@page{{size:200px 100px;margin:0}} body{{margin:0;font:20px/20px Ahem}} table{{border-spacing:0}} td{{padding:0;vertical-align:top}}</style><body>{body}</body>"
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
            for word in run.text.split_whitespace() {
                let glyph = word.chars().next().unwrap();
                tops.push((index, glyph, (run.origin.1 - run.ascent).round() as i32));
            }
        }
    }
    tops.sort_unstable();
    tops
}

/// A 50px box and, after it, a 70px box that avoids a break inside it.
const AVOIDING_PAIR: &str =
    r#"<div style="height:50px">A</div><div style="height:70px;break-inside:avoid">B</div>"#;

#[test]
fn an_avoiding_block_in_a_cell_moves_to_the_next_page() {
    for cell in [
        format!(r#"<td style="height:150px">{AVOIDING_PAIR}</td>"#),
        format!("<td>{AVOIDING_PAIR}</td>"),
    ] {
        assert_eq!(
            glyph_tops(&format!(
                "<table><tr>{cell}<td>C</td></tr></table><div>X</div>"
            )),
            [(0, 'A', 0), (0, 'C', 0), (1, 'B', 0), (1, 'X', 70)]
        );
    }
}

#[test]
fn content_after_the_moved_block_and_later_rows_follow_it() {
    assert_eq!(
        glyph_tops(&format!(
            "<table><tr><td>{AVOIDING_PAIR}<div>D</div></td><td>C</td></tr></table><div>X</div>"
        )),
        [
            (0, 'A', 0),
            (0, 'C', 0),
            (1, 'B', 0),
            (1, 'D', 70),
            (2, 'X', 0)
        ]
    );
    assert_eq!(
        glyph_tops(&format!(
            "<table><tr><td>{AVOIDING_PAIR}</td><td>C</td></tr><tr><td>R</td></tr></table><div>X</div>"
        )),
        [
            (0, 'A', 0),
            (0, 'C', 0),
            (1, 'B', 0),
            (1, 'R', 70),
            (2, 'X', 0)
        ]
    );
}

#[test]
fn a_wrapper_or_a_nested_table_in_the_cell_grows_with_it() {
    assert_eq!(
        glyph_tops(&format!(
            "<table><tr><td><div>{AVOIDING_PAIR}</div><div>D</div></td><td>C</td></tr></table><div>X</div>"
        )),
        [
            (0, 'A', 0),
            (0, 'C', 0),
            (1, 'B', 0),
            (1, 'D', 70),
            (2, 'X', 0)
        ]
    );
    assert_eq!(
        glyph_tops(&format!(
            "<table><tr><td><table><tr><td>{AVOIDING_PAIR}</td></tr></table></td><td>C</td></tr></table><div>X</div>"
        )),
        [(0, 'A', 0), (0, 'C', 0), (1, 'B', 0), (1, 'X', 70)]
    );
    // A cell of an anonymous table.
    assert_eq!(
        glyph_tops(&format!(
            r#"<div style="display:table"><div style="display:table-cell">{AVOIDING_PAIR}</div></div><div>X</div>"#
        )),
        [(0, 'A', 0), (1, 'B', 0), (1, 'X', 70)]
    );
}

#[test]
fn margins_padding_and_borders_below_the_moved_block_follow_it() {
    let expected = [(0, 'A', 0), (0, 'C', 0), (1, 'B', 0), (1, 'X', 80)];
    assert_eq!(
        glyph_tops(
            r#"<table><tr><td><div style="height:50px">A</div><div style="height:70px;break-inside:avoid;margin-bottom:10px">B</div></td><td>C</td></tr></table><div>X</div>"#
        ),
        expected
    );
    assert_eq!(
        glyph_tops(&format!(
            r#"<table><tr><td style="padding-bottom:10px">{AVOIDING_PAIR}</td><td>C</td></tr></table><div>X</div>"#
        )),
        expected
    );
    assert_eq!(
        glyph_tops(&format!(
            r#"<div style="border-bottom:10px solid"><table><tr><td>{AVOIDING_PAIR}</td><td>C</td></tr></table></div><div>X</div>"#
        )),
        expected
    );
}

#[test]
fn avoiding_blocks_in_two_cells_move_together() {
    assert_eq!(
        glyph_tops(
            r#"<table><tr><td><div style="height:50px">A</div><div style="height:70px;break-inside:avoid">B</div></td><td><div style="height:50px">C</div><div style="height:70px;break-inside:avoid">E</div></td></tr></table><div>X</div>"#
        ),
        [
            (0, 'A', 0),
            (0, 'C', 0),
            (1, 'B', 0),
            (1, 'E', 0),
            (1, 'X', 70)
        ]
    );
}

#[test]
fn an_avoiding_paragraph_or_block_of_blocks_moves_whole() {
    assert_eq!(
        glyph_tops(
            r#"<table><tr><td><div style="height:50px">A</div><div style="break-inside:avoid">B<br>D<br>E</div></td><td>C</td></tr></table><div>X</div>"#
        ),
        [
            (0, 'A', 0),
            (0, 'C', 0),
            (1, 'B', 0),
            (1, 'D', 20),
            (1, 'E', 40),
            (1, 'X', 60)
        ]
    );
    assert_eq!(
        glyph_tops(
            r#"<table><tr><td><div style="height:50px">A</div><div style="break-inside:avoid-page"><div style="height:30px">B</div><div style="height:40px">D</div></div></td><td>C</td></tr></table><div>X</div>"#
        ),
        [
            (0, 'A', 0),
            (0, 'C', 0),
            (1, 'B', 0),
            (1, 'D', 30),
            (1, 'X', 70)
        ]
    );
}

#[test]
fn a_block_taller_than_the_page_still_moves_to_its_top() {
    assert_eq!(
        glyph_tops(
            r#"<table><tr><td><div style="height:50px">A</div><div style="height:170px;break-inside:avoid">B</div></td><td>C</td></tr></table><div>X</div>"#
        ),
        [(0, 'A', 0), (0, 'C', 0), (1, 'B', 0), (2, 'X', 70)]
    );
}

#[test]
fn a_one_column_grid_or_an_anonymous_cell_moves_it_too() {
    assert_eq!(
        glyph_tops(&format!(
            r#"<table><tr><td><div style="display:grid">{AVOIDING_PAIR}</div></td><td>C</td></tr></table><div>X</div>"#
        )),
        [(0, 'A', 0), (0, 'C', 0), (1, 'B', 0), (1, 'X', 70)]
    );
    assert_eq!(
        glyph_tops(&format!(
            r#"<div style="display:table-row">{AVOIDING_PAIR}</div><div>X</div>"#
        )),
        [(0, 'A', 0), (1, 'B', 0), (1, 'X', 70)]
    );
    assert_eq!(
        glyph_tops(&format!(
            r#"<div style="display:table-row"><div>{AVOIDING_PAIR}<div>D</div></div></div><div>X</div>"#
        )),
        [(0, 'A', 0), (1, 'B', 0), (1, 'D', 70), (2, 'X', 0)]
    );
}

#[test]
fn boxes_side_by_side_in_the_cell_keep_it_in_place() {
    // Chromium leaves the block in its column or grid track.
    for container in [
        "display:grid;grid-template-columns:1fr 1fr",
        "columns:2;column-gap:0",
    ] {
        assert_eq!(
            glyph_tops(&format!(
                r#"<table><tr><td><div style="{container}">{AVOIDING_PAIR}</div></td><td>C</td></tr></table><div>X</div>"#
            )),
            [(0, 'A', 0), (0, 'B', 0), (0, 'C', 0), (0, 'X', 70)]
        );
    }
}
