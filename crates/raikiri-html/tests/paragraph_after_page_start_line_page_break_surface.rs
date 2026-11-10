//! A paragraph whose last line starts a page exactly needs no break, but the
//! paragraphs after it in the same block still break between their lines on
//! that page: a line that crosses its end starts the next page, and orphans
//! and widows move lines as on any other page. Where each line falls is what
//! Chromium prints for the same documents on pages 100px tall, with Ahem
//! lines of 20px and a line height of 30px; a `Z` block after the box shows
//! where the box ends.

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
            let Some(glyph) = run.text.trim().chars().next() else {
                continue;
            };
            tops.push((index, glyph, (run.origin.1 - run.ascent).round() as i32));
        }
    }
    tops.sort_unstable();
    tops.dedup();
    tops
}

/// The paragraphs in a block and the page, glyph and top of each line.
type Row = (&'static str, &'static [(u32, char, i32)]);

/// A first paragraph whose second line starts the second page exactly.
const FIRST: &str = "<p style='margin:0;orphans:1;widows:1'>X X</p>";

fn assert_rows(rows: &[Row]) {
    for &(after, expected) in rows {
        let body = format!(
            "<div style='height:70px'></div><div style='width:20px'>{FIRST}{after}</div><div style='height:30px'>Z</div>"
        );
        assert_eq!(glyph_tops(&body), expected, "{body}");
    }
}

#[test]
fn a_line_crossing_the_page_end_starts_the_next_page() {
    assert_rows(&[
        (
            "<p style='margin:0;orphans:1;widows:1'>X X X X</p>",
            &[
                (0, 'X', 75),
                (1, 'X', 5),
                (1, 'X', 35),
                (1, 'X', 65),
                (2, 'X', 5),
                (2, 'X', 35),
                (2, 'Z', 65),
            ],
        ),
        // So does a line of a later paragraph, and one after a box.
        (
            "<p style='margin:0;orphans:1;widows:1'>X X X</p><p style='margin:0;orphans:1;widows:1'>X X X X</p>",
            &[
                (0, 'X', 75),
                (1, 'X', 5),
                (1, 'X', 35),
                (1, 'X', 65),
                (2, 'X', 5),
                (2, 'X', 35),
                (2, 'X', 65),
                (3, 'X', 5),
                (3, 'X', 35),
                (3, 'Z', 65),
            ],
        ),
        (
            "<div style='height:5px'></div><p style='margin:0;orphans:1;widows:1'>X X X X</p>",
            &[
                (0, 'X', 75),
                (1, 'X', 5),
                (1, 'X', 40),
                (1, 'X', 70),
                (2, 'X', 5),
                (2, 'X', 35),
                (2, 'Z', 65),
            ],
        ),
    ]);
}

#[test]
fn orphans_and_widows_move_lines_on_that_page() {
    assert_rows(&[
        (
            "<p style='margin:0;orphans:3;widows:1'>X X X X</p>",
            &[
                (0, 'X', 75),
                (1, 'X', 5),
                (2, 'X', 5),
                (2, 'X', 35),
                (2, 'X', 65),
                (3, 'X', 5),
                (3, 'Z', 35),
            ],
        ),
        (
            "<p style='margin:0;orphans:1;widows:3'>X X X X</p>",
            &[
                (0, 'X', 75),
                (1, 'X', 5),
                (1, 'X', 35),
                (2, 'X', 5),
                (2, 'X', 35),
                (2, 'X', 65),
                (3, 'Z', 5),
            ],
        ),
    ]);
}
