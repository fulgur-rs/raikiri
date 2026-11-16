//! A paragraph whose `min-height` leaves room below its content, with lines
//! that pagination moves down. Once they move down by as much as that room,
//! the paragraph grows with them, and its bottom padding and border follow
//! its last box, as when its height follows its content: a last block child,
//! or the last line, moves to the next page with them. While room is left,
//! the page breaks inside it instead. The edges of a box whose `height` or
//! `max-height` ends it where it ends stay at that end, above a last block
//! child moved down past it. Where each glyph falls is what Chromium prints
//! for the same documents on pages 100px square, with Ahem glyphs of 20px on
//! lines of 30px.

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

/// The page, glyph and glyph top of every word of text in `body`, where it
/// starts.
fn glyph_tops(body: &str) -> Vec<(u32, char, i32)> {
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
    tops
}

/// A spacer `spacer` tall, then a paragraph 20px wide styled `style` holding
/// `lines` words of one line each and then `trailing`, then a block `Z`.
fn paragraph(spacer: u32, style: &str, lines: usize, trailing: &str) -> String {
    format!(
        "<div style='height:{spacer}px'></div><div style='width:20px;{style}'>{}{trailing}</div><div>Z</div>",
        vec!["X"; lines].join(" ")
    )
}

/// The glyph top of `Z` in `body`, with its page.
fn z_top(body: &str) -> (u32, i32) {
    glyph_tops(body)
        .into_iter()
        .find_map(|(page, glyph, top)| (glyph == 'Z').then_some((page, top)))
        .unwrap()
}

#[test]
fn a_last_block_moves_with_the_edges_once_moved_lines_fill_the_room() {
    let block = |height: u32| format!("<div style='height:{height}px'></div>");
    for (spacer, style, trailing, z) in [
        (0, "min-height:170px;padding-bottom:30px", 40, (2, 75)),
        (25, "min-height:170px;border-bottom:10px solid", 40, (2, 55)),
        (0, "min-height:150px;padding-bottom:30px", 20, (2, 55)),
        (0, "min-height:170px;padding-bottom:15px", 30, (2, 50)),
        // The lines fill only part of the room: the page breaks inside it.
        (0, "min-height:200px;padding-bottom:15px", 30, (2, 20)),
    ] {
        let body = paragraph(spacer, style, 4, &block(trailing));
        assert_eq!(z_top(&body), z, "{body}");
    }
}

#[test]
fn the_last_line_moves_with_the_edges_once_moved_lines_fill_the_room() {
    for (style, z) in [
        ("min-height:170px;padding-bottom:15px", 50),
        ("min-height:170px;padding-bottom:30px", 65),
    ] {
        let body = paragraph(15, style, 5, "");
        assert_eq!(
            glyph_tops(&body),
            vec![
                (0, 'X', 20),
                (0, 'X', 50),
                (1, 'X', 5),
                (1, 'X', 35),
                (2, 'X', 5),
                (2, 'Z', z)
            ],
            "{body}"
        );
    }
}

#[test]
fn edges_of_a_box_of_set_height_stay_above_a_last_block_moved_past_them() {
    for style in [
        "height:150px;padding-bottom:15px",
        "max-height:150px;padding-bottom:15px",
    ] {
        let body = paragraph(0, style, 4, "<div style='height:30px'>T</div>");
        let tops = glyph_tops(&body);
        assert!(tops.contains(&(1, 'T', 65)), "{body}: {tops:?}");
        assert!(tops.contains(&(1, 'Z', 70)), "{body}: {tops:?}");
    }
}

#[test]
fn a_max_height_lets_the_edges_follow_only_as_far_as_it_grows() {
    let last_line = |style: &str| {
        glyph_tops(&paragraph(15, style, 5, ""))
            .into_iter()
            .filter(|&(_, glyph, _)| glyph == 'X')
            .last()
            .unwrap()
    };
    // The moved lines grow the paragraph to its `max-height` or short of it:
    // the edges follow the last line to the next page.
    assert_eq!(
        last_line("min-height:170px;max-height:175px;padding-bottom:30px"),
        (2, 'X', 5)
    );
    assert_eq!(
        last_line("min-height:170px;max-height:180px;padding-bottom:30px"),
        (2, 'X', 5)
    );
    // They would grow it past its `max-height`: the edges stay at its end,
    // which fits the page, and the last line stays.
    assert_eq!(
        last_line("min-height:155px;max-height:160px;padding-bottom:15px"),
        (1, 'X', 65)
    );
}
