//! Forced column breaks in a multicol container of `auto` height that need
//! more columns than its `column-count`: in print, the columns past the last
//! one go on on the next page, as many pages as they need, even when the
//! content would fit the first page. Where each line falls is what Chromium
//! prints for the same documents on pages 200px square, in columns of Ahem
//! lines 20px tall, each paragraph 20px wide so that every letter takes a
//! line.

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

/// Words of five glyphs, one to a line of a column: the `first`th to the
/// `last`th letter of the alphabet.
fn w(first: u8, last: u8) -> String {
    (first..=last)
        .map(|index| char::from(b'A' + index - 1).to_string().repeat(5))
        .collect::<Vec<_>>()
        .join(" ")
}

/// The columns of `body`: for each page and column left edge, the top of its
/// first line and the glyph of each line, top to bottom.
fn columns(body: &str) -> Vec<(u32, i32, i32, String)> {
    let fonts = FontCollectionBuilder::new()
        .font_bytes(
            "Ahem",
            include_bytes!("../../raikiri-dom/tests/data/text-autospace/Ahem.ttf").to_vec(),
        )
        .build()
        .unwrap();
    let resources = RenderResources::new().fonts(fonts);
    let html = format!(
        "<!doctype html><style>@page{{size:200px 200px;margin:0}} body{{margin:0;font:20px/20px Ahem}} .mc{{columns:2;column-gap:0}} p{{margin:0;width:20px}}</style><body>{body}</body>"
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
        let page = result.page(index).unwrap();
        for run in page.text_runs() {
            let top = (run.origin.1 - run.ascent).round() as i32;
            if let Some(glyph) = run.text.chars().find(|glyph| !glyph.is_whitespace()) {
                lines.push((index, run.origin.0.round() as i32, top, glyph));
            }
        }
    }
    lines.sort_unstable();
    let mut columns: Vec<(u32, i32, i32, String)> = Vec::new();
    for (page, x, top, glyph) in lines {
        match columns.last_mut() {
            Some(column) if (column.0, column.1) == (page, x) => {
                assert_eq!(column.2 + 20 * column.3.len() as i32, top, "{body}");
                column.3.push(glyph);
            }
            _ => columns.push((page, x, top, glyph.to_string())),
        }
    }
    columns
}

fn assert_columns(body: &str, expected: &[(u32, i32, i32, &str)]) {
    let expected: Vec<_> = expected
        .iter()
        .map(|&(page, x, top, glyphs)| (page, x, top, glyphs.to_string()))
        .collect();
    assert_eq!(columns(body), expected, "{body}");
}

/// Five paragraphs of two lines, each after the first starting a column.
const FIVE: &str = "<p>A B</p><p style=break-before:column>C D</p><p style=break-before:column>E F</p><p style=break-before:column>G H</p><p style=break-before:column>I J</p>";

#[test]
fn columns_past_the_last_one_go_on_on_the_next_page() {
    let expected = [
        (0, 0, 0, "AB"),
        (0, 100, 0, "CD"),
        (1, 0, 0, "EF"),
        (1, 100, 0, "GH"),
        (2, 0, 0, "IJ"),
    ];
    assert_columns(&format!("<div class=mc>{FIVE}</div>"), &expected);
    // Breaks after the paragraphs do the same.
    assert_columns(
        "<div class=mc><p style=break-after:column>A B</p><p style=break-after:column>C D</p>\
         <p style=break-after:column>E F</p><p>G H</p></div>",
        &expected[..4],
    );
    // So do breaks inside wrapping blocks.
    assert_columns(
        "<div class=mc><div><p>A B</p><p style=break-before:column>C D</p></div>\
         <div style=break-before:column><p>E F</p><p style=break-before:column>G H</p></div>\
         <p style=break-before:column>I J</p></div>",
        &expected,
    );
}

#[test]
fn content_around_the_container_follows_its_pages() {
    // What follows the container comes after its last column.
    assert_columns(
        &format!("<div class=mc>{FIVE}</div><p>K</p>"),
        &[
            (0, 0, 0, "AB"),
            (0, 100, 0, "CD"),
            (1, 0, 0, "EF"),
            (1, 100, 0, "GH"),
            (2, 0, 0, "IJK"),
        ],
    );
    // Content before it leaves the first page's columns below it.
    assert_columns(
        &format!("<p>K</p><div class=mc>{FIVE}</div>"),
        &[
            (0, 0, 0, "KAB"),
            (0, 100, 20, "CD"),
            (1, 0, 0, "EF"),
            (1, 100, 0, "GH"),
            (2, 0, 0, "IJ"),
        ],
    );
    // A container that starts low on the page.
    assert_columns(
        &format!("{}<div class=mc>{FIVE}</div>", "<p>Z</p>".repeat(8)),
        &[
            (0, 0, 0, "ZZZZZZZZAB"),
            (0, 100, 160, "CD"),
            (1, 0, 0, "EF"),
            (1, 100, 0, "GH"),
            (2, 0, 0, "IJ"),
        ],
    );
}

#[test]
fn more_columns_and_unbroken_content_fill_each_page_in_turn() {
    // Three columns take three paragraphs a page.
    assert_columns(
        &format!("<div class=mc style=columns:3>{FIVE}</div>"),
        &[
            (0, 0, 0, "AB"),
            (0, 67, 0, "CD"),
            (0, 133, 0, "EF"),
            (1, 0, 0, "GH"),
            (1, 67, 0, "IJ"),
        ],
    );
    // A column taller than the others keeps its lines.
    assert_columns(
        "<div class=mc><p>A B C</p><p style=break-before:column>D E F G H I J K</p>\
         <p style=break-before:column>L M</p></div>",
        &[(0, 0, 0, "ABC"), (0, 100, 0, "DEFGHIJK"), (1, 0, 0, "LM")],
    );
    assert_columns(
        "<div class=mc><p>A B C D</p><p style=break-before:column>E F</p>\
         <p style=break-before:column>G H</p><p style=break-before:column>I J</p></div>",
        &[
            (0, 0, 0, "ABCD"),
            (0, 100, 0, "EF"),
            (1, 0, 0, "GH"),
            (1, 100, 0, "IJ"),
        ],
    );
}

#[test]
fn the_border_and_padding_of_the_container_stay_on_its_first_page() {
    assert_columns(
        &format!("<div class=mc style='border:5px solid;padding:5px'>{FIVE}</div>"),
        &[
            (0, 10, 10, "AB"),
            (0, 100, 10, "CD"),
            (1, 10, 0, "EF"),
            (1, 100, 0, "GH"),
            (2, 10, 0, "IJ"),
        ],
    );
}
