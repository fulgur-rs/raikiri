//! The first row of a row flex container or of a grid that cannot be split:
//! a monolithic item, or an item whose only line is generated text or an
//! inline-block and keeps the padding below it. When it crosses the end of a
//! page after other content, the container moves to the next page with it,
//! also when the container is the first box of plain blocks around it, which
//! go along with their padding. A grid of one column breaks between its top
//! padding and its first item instead, as a column flex container does.
//! Where each glyph falls is what Chromium prints for the same documents on
//! pages 100px square, with Ahem glyphs of 20px on lines of 30px.

use raikiri_html::{
    DocumentLayout, FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

/// Lays out `body` on the pages and with the font of the Chromium documents.
/// The paragraphs of class `g` hold one line of generated text.
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
        "<!doctype html><style>@page{{size:100px 100px;margin:0}} body{{margin:0;font:20px/30px Ahem}} .g::before{{content:\"A\"}}</style><body>{body}</body>"
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

/// The page, glyph and glyph top of every word of text in `body`.
fn glyph_tops(body: &str) -> Vec<(u32, char, i32)> {
    let result = lay_out(body);
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

const ROW_FLEX: &str = "display:flex";
const TWO_COLUMNS: &str = "display:grid;grid-template-columns:20px 20px";
const ONE_COLUMN: &str = "display:grid";

const GENERATED: &str = r#"<p class="g" style="margin:0;width:20px;padding-bottom:60px"></p>"#;
const INLINE_BLOCK: &str = r#"<p style="margin:0;width:20px;padding-bottom:60px"><span style="display:inline-block">A</span></p>"#;

#[test]
fn a_wrapped_container_moves_with_its_first_row() {
    for container in [ROW_FLEX, TWO_COLUMNS] {
        for item in [GENERATED, INLINE_BLOCK] {
            for (open, close) in [
                ("<div>", "</div>"),
                ("<div><div>", "</div></div>"),
                (r#"<div style="margin-top:10px">"#, "</div>"),
            ] {
                let body = format!(
                    r#"<div style="height:20px">Y</div>{open}<div style="{container}">{item}</div>{close}<div>Z</div>"#
                );
                assert_eq!(
                    glyph_tops(&body),
                    [(0, 'Y', 5), (1, 'A', 5), (2, 'Z', 5)],
                    "{body}"
                );
            }
        }
    }
}

#[test]
fn the_padding_of_the_wrapper_goes_along() {
    for container in [ROW_FLEX, TWO_COLUMNS, ONE_COLUMN] {
        for item in [GENERATED, INLINE_BLOCK] {
            let body = format!(
                r#"<div style="height:20px">Y</div><div style="padding-top:5px"><div style="{container}">{item}</div></div><div>Z</div>"#
            );
            assert_eq!(
                glyph_tops(&body),
                [(0, 'Y', 5), (1, 'A', 10), (2, 'Z', 5)],
                "{body}"
            );
        }
    }
}

#[test]
fn a_size_contained_item_moves_its_row() {
    for container in [ROW_FLEX, TWO_COLUMNS] {
        for (open, close) in [("", ""), ("<div>", "</div>")] {
            let body = format!(
                r#"<div style="height:40px">Y</div>{open}<div style="{container}"><div style="contain:size;width:20px;height:70px"></div></div>{close}<div>Z</div>"#
            );
            assert_eq!(glyph_tops(&body), [(0, 'Y', 5), (1, 'Z', 75)], "{body}");
        }
    }
}

#[test]
fn a_padded_grid_breaks_after_its_padding_before_a_row_that_does_not_fit() {
    // The row of two columns goes on below the padding, which stays.
    let body = format!(
        r#"<div style="height:30px">Y</div><div style="{TWO_COLUMNS};padding-top:60px"><div style="contain:size;width:20px;height:20px"></div><div style="width:20px">A</div></div><div>Z</div>"#
    );
    assert_eq!(
        glyph_tops(&body),
        [(0, 'Y', 5), (1, 'A', 5), (1, 'Z', 35)],
        "{body}"
    );
    // A grid of one column stacks its items: the first goes on below the
    // padding and the next follows it.
    for (height, expected) in [
        (20, [(0, 'Y', 5), (1, 'A', 25), (1, 'Z', 55)]),
        (70, [(0, 'Y', 5), (1, 'A', 75), (2, 'Z', 5)]),
    ] {
        let body = format!(
            r#"<div style="height:30px">Y</div><div style="{ONE_COLUMN};padding-top:60px"><div style="contain:size;width:20px;height:{height}px"></div><div style="width:20px">A</div></div><div>Z</div>"#
        );
        assert_eq!(glyph_tops(&body), expected, "{body}");
    }
}
