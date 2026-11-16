//! A forced page break inside a table cell whose `height` is set. The cell's
//! `height` is only a minimum (CSS 2.1 §17.5.3): the content the break moves
//! to the next page takes the room that height left below it first, and the
//! row grows only as far as the moved content ends past it. The flow after
//! the table follows the row's end, and the cells beside the broken one keep
//! their places. Where each glyph falls is what Chromium prints for the same
//! documents on pages 200px wide and 100px tall, with Ahem glyphs of 20px on
//! lines of 20px.

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

/// A cell `height` tall holding a 20px box and, after a forced break, a box
/// `moved` tall.
fn broken_cell(height: u32, moved: u32) -> String {
    format!(
        r#"<td style="height:{height}px"><div style="height:20px">A</div><div style="height:{moved}px;break-before:page">B</div></td>"#
    )
}

#[test]
fn the_flow_after_a_sized_cell_follows_the_moved_content() {
    // The moved box ends at 170px, 20px past the cell's 150px.
    assert_eq!(
        glyph_tops(&format!(
            "<table><tr>{}</tr></table><div>X</div>",
            broken_cell(150, 70)
        )),
        [(0, 'A', 0), (1, 'B', 0), (1, 'X', 70)]
    );
    // The moved box ends past the next page too.
    assert_eq!(
        glyph_tops(&format!(
            "<table><tr>{}</tr></table><div>X</div>",
            broken_cell(150, 120)
        )),
        [(0, 'A', 0), (1, 'B', 0), (2, 'X', 20)]
    );
    // A cell shorter than the page ends with the moved box.
    assert_eq!(
        glyph_tops(&format!(
            "<table><tr>{}</tr></table><div>X</div>",
            broken_cell(60, 20)
        )),
        [(0, 'A', 0), (1, 'B', 0), (1, 'X', 20)]
    );
}

#[test]
fn a_row_height_is_a_minimum_too() {
    assert_eq!(
        glyph_tops(
            r#"<table><tr style="height:150px"><td><div style="height:20px">A</div><div style="height:70px;break-before:page">B</div></td></tr></table><div>X</div>"#
        ),
        [(0, 'A', 0), (1, 'B', 0), (1, 'X', 70)]
    );
}

#[test]
fn the_rows_after_a_sized_cell_follow_the_moved_content() {
    assert_eq!(
        glyph_tops(&format!(
            "<table><tr>{}</tr><tr><td>Y</td></tr></table><div>X</div>",
            broken_cell(150, 70)
        )),
        [(0, 'A', 0), (1, 'B', 0), (1, 'Y', 70), (2, 'X', 0)]
    );
}

#[test]
fn the_cells_beside_a_sized_cell_keep_their_places() {
    assert_eq!(
        glyph_tops(&format!(
            "<table><tr>{}<td>C</td></tr></table><div>X</div>",
            broken_cell(150, 70)
        )),
        [(0, 'A', 0), (0, 'C', 0), (1, 'B', 0), (1, 'X', 70)]
    );
    assert_eq!(
        glyph_tops(&format!(
            r#"<div style="border:solid 5px"><table><tr>{}<td>C</td></tr><tr><td>D</td><td>E</td></tr></table></div><div>X</div>"#,
            broken_cell(150, 70)
        )),
        [
            (0, 'A', 5),
            (0, 'C', 5),
            (1, 'B', 0),
            (1, 'D', 70),
            (1, 'E', 70),
            (2, 'X', 0)
        ]
    );
}

#[test]
fn cells_in_an_anonymous_row_break_the_same_way() {
    const CELL: &str = r#"<div style="display:table-cell;height:150px"><div style="height:20px">A</div><div style="height:70px;break-before:page">B</div></div>"#;
    // The cells of a table without rows share an anonymous row.
    assert_eq!(
        glyph_tops(&format!(
            r#"<div style="display:table">{CELL}<div style="display:table-cell">C</div></div><div>X</div>"#
        )),
        [(0, 'A', 0), (0, 'C', 0), (1, 'B', 0), (1, 'X', 70)]
    );
    // A cell without a table gets an anonymous table around it.
    assert_eq!(
        glyph_tops(&format!("{CELL}<div>X</div>")),
        [(0, 'A', 0), (1, 'B', 0), (1, 'X', 70)]
    );
}

#[test]
fn the_content_after_the_moved_box_grows_the_row_too() {
    // The cell's content ends at 210px, so the row does.
    assert_eq!(
        glyph_tops(
            r#"<table><tr><td style="height:150px"><div style="height:20px">A</div><div style="height:70px;break-before:page">B</div><div style="height:40px">C</div></td></tr></table><div>X</div>"#
        ),
        [(0, 'A', 0), (1, 'B', 0), (1, 'C', 70), (2, 'X', 10)]
    );
}

#[test]
fn the_cells_beside_keep_their_places_in_any_row() {
    let expected = [(0, 'A', 0), (0, 'C', 0), (1, 'B', 0), (1, 'X', 70)];
    // An anonymous row in a row group.
    assert_eq!(
        glyph_tops(
            r#"<div style="display:table"><div style="display:table-row-group"><div style="display:table-cell;height:150px"><div style="height:20px">A</div><div style="height:70px;break-before:page">B</div></div><div style="display:table-cell">C</div></div></div><div>X</div>"#
        ),
        expected
    );
    // A row with no box of its own.
    assert_eq!(
        glyph_tops(&format!(
            r#"<table><tr style="display:contents">{}<td>C</td></tr></table><div>X</div>"#,
            broken_cell(150, 70)
        )),
        expected
    );
    // A cell with an auto height grows by the whole move.
    assert_eq!(
        glyph_tops(
            r#"<table><tr><td><div style="height:20px">A</div><div style="height:70px;break-before:page">B</div></td><td>C</td></tr></table><div>X</div>"#
        ),
        expected
    );
}

#[test]
fn a_sized_ancestor_of_the_table_does_not_grow() {
    assert_eq!(
        glyph_tops(&format!(
            r#"<div style="height:300px"><table><tr>{}</tr></table></div><div>X</div>"#,
            broken_cell(150, 70)
        )),
        [(0, 'A', 0), (1, 'B', 0), (3, 'X', 0)]
    );
}

#[test]
fn the_moved_box_keeps_its_bottom_margin_in_a_formatting_context_root() {
    // The margin ends at 190px, so the line after it does not fit below.
    let expected = [(0, 'A', 0), (1, 'B', 0), (2, 'X', 0)];
    const MOVED: &str = r#"<div style="height:70px;break-before:page;margin-bottom:20px">B</div>"#;
    for (name, root) in [
        (
            "cell",
            r#"<table><tr><td style="height:150px">{}</td></tr></table>"#,
        ),
        (
            "flow root",
            r#"<div style="display:flow-root;min-height:150px">{}</div>"#,
        ),
    ] {
        for (wrap, content) in [
            ("direct", MOVED.to_string()),
            ("wrapped", format!("<div>{MOVED}</div>")),
        ] {
            let body = root.replace(
                "{}",
                &format!(r#"<div style="height:20px">A</div>{content}"#),
            );
            assert_eq!(
                glyph_tops(&format!("{body}<div>X</div>")),
                expected,
                "{name} {wrap}"
            );
        }
    }
}

#[test]
fn a_cell_tall_enough_for_the_moved_content_does_not_grow() {
    // The moved box ends at 170px, inside the cell's 250px.
    assert_eq!(
        glyph_tops(&format!(
            "<table><tr>{}<td>C</td></tr></table><div>X</div>",
            broken_cell(250, 70)
        )),
        [(0, 'A', 0), (0, 'C', 0), (1, 'B', 0), (2, 'X', 50)]
    );
    assert_eq!(
        glyph_tops(
            r#"<div style="display:table-cell;height:250px"><div style="height:20px">A</div><div style="height:70px;break-before:page">B</div></div><div>X</div>"#
        ),
        [(0, 'A', 0), (1, 'B', 0), (2, 'X', 50)]
    );
}

#[test]
fn a_row_grows_once_for_several_breaks() {
    const TWO_BREAKS: &str = r#"<div style="height:20px">A</div><div style="height:70px;break-before:page">B</div><div style="height:20px;break-before:page">C</div>"#;
    // Two breaks in one cell: its content ends at 220px.
    let expected = [(0, 'A', 0), (1, 'B', 0), (2, 'C', 0), (2, 'X', 20)];
    assert_eq!(
        glyph_tops(&format!(
            r#"<table><tr><td style="height:100px">{TWO_BREAKS}</td></tr></table><div>X</div>"#
        )),
        expected
    );
    assert_eq!(
        glyph_tops(&format!(
            r#"<div style="display:table-cell;height:100px">{TWO_BREAKS}</div><div>X</div>"#
        )),
        expected
    );
    // Breaks in two cells of a row: the row grows as far as the cell that
    // grows most.
    let second = r#"<td style="height:150px"><div style="height:20px">D</div><div style="height:60px;break-before:page">E</div></td>"#;
    let expected = [
        (0, 'A', 0),
        (0, 'D', 0),
        (1, 'B', 0),
        (1, 'E', 0),
        (1, 'X', 70),
    ];
    assert_eq!(
        glyph_tops(&format!(
            "<table><tr>{}{second}</tr></table><div>X</div>",
            broken_cell(150, 70)
        )),
        expected
    );
    assert_eq!(
        glyph_tops(&format!(
            "<table><tr>{}{}</tr></table><div>X</div>",
            broken_cell(150, 70).replace(r#" style="height:150px""#, ""),
            second.replace(r#" style="height:150px""#, "")
        )),
        expected
    );
}

#[test]
fn each_anonymous_row_grows_on_its_own() {
    let cell = |first: char, second: char| {
        format!(
            r#"<div style="display:table-cell;height:150px"><div style="height:20px">{first}</div><div style="height:70px;break-before:page">{second}</div></div>"#
        )
    };
    // Two runs of cells around a row make two anonymous rows; each grows
    // 20px past its cells' height.
    assert_eq!(
        glyph_tops(&format!(
            r#"<div style="display:table">{}<div style="display:table-row"><div style="display:table-cell;height:30px">Y</div></div>{}</div><div>X</div>"#,
            cell('A', 'B'),
            cell('D', 'E')
        )),
        [
            (0, 'A', 0),
            (1, 'B', 0),
            (1, 'Y', 70),
            (2, 'D', 0),
            (3, 'E', 0),
            (3, 'X', 70)
        ]
    );
}

#[test]
fn text_after_the_moved_box_keeps_its_margin_in_the_wrapper() {
    // The margin sits between B and the text, inside the wrapper, and
    // counts once: the wrapper ends at 190px.
    for trailing in ["T", "<span>T</span>"] {
        assert_eq!(
            glyph_tops(&format!(
                r#"<table><tr><td style="height:150px"><div style="height:20px">A</div><div><div style="height:50px;break-before:page;margin-bottom:20px">B</div>{trailing}</div></td></tr></table><div>X</div>"#
            )),
            [(0, 'A', 0), (1, 'B', 0), (1, 'T', 70), (2, 'X', 0)],
            "{trailing}"
        );
    }
    // A moved box after the text still ends the wrapper with its margin.
    assert_eq!(
        glyph_tops(
            r#"<table><tr><td style="height:150px"><div style="height:20px">A</div><div>T<div style="height:50px;break-before:page;margin-bottom:20px">B</div></div></td></tr></table><div>X</div>"#
        ),
        [(0, 'A', 0), (0, 'T', 20), (1, 'B', 0), (1, 'X', 70)]
    );
}

#[test]
fn a_nested_table_passes_on_all_its_growth() {
    // The inner cell's content ends at 220px after two breaks, and the
    // outer cell, row or anonymous table grows with all of it.
    const INNER: &str = r#"<table><tr><td style="height:150px"><div style="height:20px">A</div><div style="height:70px;break-before:page">B</div><div style="height:20px;break-before:page">C</div></td></tr></table>"#;
    let expected = [(0, 'A', 0), (1, 'B', 0), (2, 'C', 0), (2, 'X', 20)];
    for outer in [
        "<table><tr><td>{}</td></tr></table>",
        r#"<table><tr><td style="height:120px">{}</td></tr></table>"#,
        r#"<div style="display:table-cell;height:120px">{}</div>"#,
    ] {
        assert_eq!(
            glyph_tops(&format!("{}<div>X</div>", outer.replace("{}", INNER))),
            expected,
            "{outer}"
        );
    }
}

#[test]
fn a_wrapper_sized_by_its_height_limits_keeps_the_margin_inside() {
    let moved = |limit: &str| {
        format!(
            r#"<table><tr><td style="height:150px"><div style="height:20px">A</div><div style="{limit};break-before:page"><div style="height:50px;margin-bottom:20px">B</div></div></td></tr></table><div>X</div>"#
        )
    };
    // The wrapper's `min-height` or `max-height` sets where it ends.
    assert_eq!(
        glyph_tops(&moved("min-height:60px")),
        [(0, 'A', 0), (1, 'B', 0), (1, 'X', 60)]
    );
    assert_eq!(
        glyph_tops(&moved("max-height:40px")),
        [(0, 'A', 0), (1, 'B', 0), (1, 'X', 50)]
    );
    // A `min-height` below the content leaves the margin to collapse.
    assert_eq!(
        glyph_tops(&moved("min-height:10px")),
        [(0, 'A', 0), (1, 'B', 0), (1, 'X', 70)]
    );
}
