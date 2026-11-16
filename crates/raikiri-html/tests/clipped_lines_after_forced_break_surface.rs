//! Lines inside a box with a fixed height that clips its overflow, after a
//! forced break inside that box. The break moves the lines down, but the box
//! keeps its height and ends where it did, so the lines moved past its end are
//! hidden: they reach no page of their own, and the page count is the one
//! Chromium prints for the same documents on pages 100px square, with Ahem
//! glyphs of 20px on lines of 30px.

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

/// The page count, then the page and glyph top of each glyph of `glyphs`
/// in `body`, where it starts.
fn pages_and_tops(body: &str, glyphs: &str) -> (u32, Vec<(u32, char, i32)>) {
    let result = lay_out(body);
    let mut tops = Vec::new();
    for index in 0..result.page_count() {
        let page = result.page(index).unwrap();
        for run in page.text_runs() {
            for word in run.text.split_whitespace() {
                let glyph = word.chars().next().unwrap();
                let top = (run.origin.1 - run.ascent).round() as i32;
                if glyphs.contains(glyph) && top >= 0 {
                    tops.push((index, glyph, top));
                }
            }
        }
    }
    tops.sort_unstable();
    (result.page_count(), tops)
}

/// A box `style` that clips, holding a block `A` 20px tall and then, after a
/// forced break, the lines `lines`, followed by a block `X`.
fn clipped(style: &str, lines: &str) -> String {
    let lines = lines
        .chars()
        .map(String::from)
        .collect::<Vec<_>>()
        .join("<br>");
    format!(
        "<div style='{style}'><div style='height:20px'>A</div>\
         <div style='break-before:page'>{lines}</div></div><div>X</div>"
    )
}

#[test]
fn lines_moved_past_the_end_of_a_clipping_box_add_no_page() {
    for overflow in ["hidden", "clip", "auto"] {
        for (height, lines) in [(130, "abcde"), (150, "abcdefgh"), (150, "abcdefghijkl")] {
            let body = clipped(&format!("height:{height}px;overflow:{overflow}"), lines);
            assert_eq!(
                pages_and_tops(&body, "AXa"),
                (2, vec![(0, 'A', 5), (1, 'X', height - 95), (1, 'a', 5)]),
                "{body}"
            );
        }
        // The box runs over a second page; the lines past its end on the
        // third add no fourth.
        let body = clipped(&format!("height:250px;overflow:{overflow}"), "abcdefghijkl");
        assert_eq!(
            pages_and_tops(&body, "AXad"),
            (3, vec![(0, 'A', 5), (1, 'a', 5), (2, 'X', 55), (2, 'd', 5)]),
            "{body}"
        );
    }
}

#[test]
fn the_clip_of_a_box_around_the_paragraph_hides_the_lines_it_moved() {
    for style in [
        "max-height:150px;overflow:hidden",
        "height:130px;padding-bottom:20px;overflow:hidden",
    ] {
        let body = clipped(style, "abcdefgh");
        assert_eq!(
            pages_and_tops(&body, "AXa"),
            (2, vec![(0, 'A', 5), (1, 'X', 55), (1, 'a', 5)]),
            "{body}"
        );
    }
    let nested = "<div style='height:150px;overflow:hidden'><div>\
                  <div style='height:20px'>A</div>\
                  <div style='break-before:page'>a<br>b<br>c<br>d<br>e<br>f<br>g<br>h</div>\
                  </div></div><div>X</div>";
    assert_eq!(
        pages_and_tops(nested, "AXa"),
        (2, vec![(0, 'A', 5), (1, 'X', 55), (1, 'a', 5)])
    );
    let wrapped = "<div style='height:140px;overflow:hidden'>\
                   <div style='height:20px'>A</div>\
                   <div style='break-before:page;width:20px'>a b c d e f g h</div>\
                   </div><div>X</div>";
    assert_eq!(
        pages_and_tops(wrapped, "AXa"),
        (2, vec![(0, 'A', 5), (1, 'X', 45), (1, 'a', 5)])
    );
}

#[test]
fn a_box_whose_height_follows_its_lines_still_takes_their_pages() {
    let body = clipped("height:150px", "abcdefgh");
    let (pages, _) = pages_and_tops(&body, "");
    assert_eq!(pages, 4, "{body}");
}
