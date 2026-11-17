//! A paragraph of set height, `max-height` or `min-height` whose lines
//! pagination moves down, with a block child after its last line. The block
//! moves down with the lines, and the content after it moves only that far:
//! no blank page follows for content that ends where the block does. The
//! pages and glyph tops are what Chromium prints for the same documents on
//! pages 100px square, with Ahem glyphs of 20px on lines of 30px.

use raikiri_html::{
    DocumentLayout, FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

/// Lays out `body` on the pages and with the font of the Chromium documents.
fn lay_out(body: &str) -> DocumentLayout {
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
    result
}

/// The page count of `body`, and the page, glyph and glyph top of every word
/// of text in it.
fn pages_and_tops(body: &str) -> (u32, Vec<(u32, char, i32)>) {
    let result = lay_out(body);
    let mut tops = Vec::new();
    for index in 0..result.page_count() {
        let page = result.page(index).unwrap();
        for run in page.text_runs() {
            for word in run.text.split_whitespace() {
                let top = (run.origin.1 - run.ascent).round() as i32;
                if top >= 0 {
                    tops.push((index, word.chars().next().unwrap(), top));
                }
            }
        }
    }
    tops.sort_unstable();
    (result.page_count(), tops)
}

#[test]
fn no_blank_page_follows_a_trailing_block_moved_with_the_lines() {
    for (spacer, style, height, block, pages, z) in [
        (0, "height:150px;padding-bottom:15px", 40, "", 2, (1, 70)),
        (0, "height:150px;padding-bottom:15px", 30, "T", 2, (1, 70)),
        (
            0,
            "height:150px;border-bottom:10px solid",
            30,
            "T",
            2,
            (1, 65),
        ),
        (50, "height:150px;padding-bottom:15px", 30, "T", 3, (2, 70)),
        (
            0,
            "max-height:120px;padding-bottom:30px",
            20,
            "",
            2,
            (1, 55),
        ),
        (
            50,
            "max-height:120px;padding-bottom:30px",
            20,
            "",
            3,
            (2, 55),
        ),
    ] {
        let body = format!(
            "<div style='height:{spacer}px'></div><div style='width:20px;{style}'>X X X X<div style='height:{height}px'>{block}</div></div><div>Z</div>"
        );
        let (count, tops) = pages_and_tops(&body);
        assert_eq!(count, pages, "{body}: {tops:?}");
        assert!(tops.contains(&(z.0, 'Z', z.1)), "{body}: {tops:?}");
    }
}

#[test]
fn the_lines_after_a_moved_block_child_move_as_far_as_it_does() {
    // A block between the lines of an auto-height paragraph keeps the lines
    // after it right below it.
    let body = "<div style='width:20px;padding-bottom:15px'>X X X X<div style='height:30px'>T</div>Y</div><div>Z</div>";
    let (count, tops) = pages_and_tops(body);
    assert_eq!(count, 3, "{tops:?}");
    assert_eq!(
        tops,
        vec![
            (0, 'X', 5),
            (0, 'X', 35),
            (1, 'T', 65),
            (1, 'X', 5),
            (1, 'X', 35),
            (2, 'Y', 5),
            (2, 'Z', 50),
        ]
    );
}
