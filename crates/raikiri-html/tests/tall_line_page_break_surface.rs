//! A line that does not start its page moves to the next page whole when it
//! crosses the page end, however tall it is: a line of text, a line of a
//! `<br>` alone, and a line holding only an inline-block, inline flex
//! container, inline grid or inline table, whose box no page break goes
//! through. The same holds for a flex container whose first item is taller
//! than a page. What does not fit the next page either runs past its end.
//! Where each glyph falls is what Chromium prints for the same documents on
//! pages 100px tall, with Ahem lines of 20px and a line height of 30px; only
//! the glyphs printed wholly inside a page are compared.

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
fn a_line_of_atomic_inlines_taller_than_a_page_starts_the_next_page() {
    // Inline flex containers (a wrapping column, a column), an inline grid,
    // an inline-block, an inline table, an inline-block inside a paragraph
    // and two inline-blocks on one line, each taller than a page. One that
    // starts its page, or that fits the page it is on, stays.
    assert_rows(&[
        (
            "<div style='height:150px'></div><div style='display:inline-flex;width:40px;flex-direction:column;flex-wrap:wrap;max-height:120px'><p style='margin:0;width:20px'>X X X</p><p style='margin:0;width:20px'>Z</p><p style='margin:0;width:20px'>W</p><p style='margin:0;width:20px'>Y Y Y</p></div>",
            &[
                (2, 'W', 5),
                (2, 'X', 5),
                (2, 'X', 35),
                (2, 'X', 65),
                (2, 'Y', 35),
                (2, 'Y', 65),
            ],
        ),
        (
            "<div style='height:150px'></div><div style='display:inline-flex;width:40px;flex-direction:column'><p style='margin:0;width:20px'>X X X</p><p style='margin:0;width:20px'>Z</p></div>",
            &[(2, 'X', 5), (2, 'X', 35), (2, 'X', 65)],
        ),
        (
            "<div style='height:150px'></div><div style='display:inline-grid;width:40px'><p style='margin:0;width:20px'>X X X</p><p style='margin:0;width:20px'>Z</p></div>",
            &[(2, 'X', 5), (2, 'X', 35), (2, 'X', 65)],
        ),
        (
            "<div style='height:150px'></div><div style='display:inline-block;width:40px'><p style='margin:0;width:20px'>X X X</p><p style='margin:0;width:20px'>Z</p></div>",
            &[(2, 'X', 5), (2, 'X', 35), (2, 'X', 65)],
        ),
        (
            "<div style='height:150px'></div><div style='display:inline-table;width:40px'><p style='margin:0;width:20px'>X X X</p><p style='margin:0;width:20px'>Z</p></div>",
            &[(2, 'X', 5), (2, 'X', 35), (2, 'X', 65)],
        ),
        (
            "<div style='height:150px'></div><p style='margin:0'><span style='display:inline-block;width:20px'><span style='display:block'>X X X</span><span style='display:block'>Z</span></span></p>",
            &[(2, 'X', 5), (2, 'X', 35), (2, 'X', 65)],
        ),
        (
            "<div style='height:150px'></div><div style='display:inline-block;width:20px'><p style='margin:0'>X X X</p><p style='margin:0'>Z</p></div><div style='display:inline-block;width:20px'><p style='margin:0'>Y</p></div>",
            &[(2, 'X', 5), (2, 'X', 35), (2, 'X', 65)],
        ),
        (
            "<div style='display:inline-flex;width:40px;flex-direction:column'><p style='margin:0;width:20px'>X X X X X</p></div>",
            &[(0, 'X', 5), (0, 'X', 35), (0, 'X', 65), (1, 'X', 25)],
        ),
        (
            "<div style='height:150px'></div><div style='display:inline-flex;width:40px;flex-direction:column'><p style='margin:0;width:20px'>X X</p></div>",
            &[(2, 'X', 5), (2, 'X', 35)],
        ),
    ]);
}

#[test]
fn a_line_of_text_taller_than_a_page_starts_the_next_page() {
    // A line of text or of a `<br>` alone, in a block of its own or of a
    // fixed height, as the first line of a paragraph with more after it,
    // below a short box or a tall one. One that starts its page stays.
    assert_rows(&[
        (
            "<div style='height:150px'></div><div style='line-height:120px'>X</div><div>Z</div>",
            &[(2, 'X', 50), (3, 'Z', 25)],
        ),
        (
            "<div style='height:8px'></div><div style='height:250px;line-height:250px'>X</div><div>Z</div>",
            &[(2, 'X', 15), (3, 'Z', 55)],
        ),
        (
            "<div style='height:8px'></div><div style='line-height:120px'>X</div><div>Z</div>",
            &[(1, 'X', 50), (2, 'Z', 25)],
        ),
        (
            "<div style='height:150px'></div><div><span style='line-height:120px'>X</span><br>Y</div>",
            &[(2, 'X', 50), (3, 'Y', 25)],
        ),
        (
            "<div style='height:150px'></div><div style='orphans:2'><span style='line-height:120px'>X</span><br>Y<br>Y</div>",
            &[(2, 'X', 50), (3, 'Y', 25), (3, 'Y', 55)],
        ),
        (
            "<div style='height:150px'></div><div style='line-height:120px'><br></div><div>Z</div>",
            &[(3, 'Z', 25)],
        ),
        (
            "<div style='line-height:120px'><br></div><div>Z</div>",
            &[(1, 'Z', 25)],
        ),
    ]);
}

#[test]
fn a_flex_container_whose_first_item_is_taller_than_a_page_starts_the_next_page() {
    // A size-contained item, an image, an image beside a paragraph, a line
    // taller than the page with and without a border above it, and an item
    // that avoids breaks inside it below the container's padding.
    assert_rows(&[
        (
            "<div style='height:150px'></div><div style='display:flex'><div style='width:20px;contain:size;height:120px'><p style='margin:0'>X X X X</p></div></div><div>Z</div>",
            &[(2, 'X', 5), (2, 'X', 35), (2, 'X', 65), (3, 'Z', 25)],
        ),
        (
            "<div style='height:150px'></div><div style='display:flex'><img style='width:20px;height:120px'></div><div>Z</div>",
            &[(3, 'Z', 25)],
        ),
        (
            "<div style='height:150px'></div><div style='display:flex'><img style='width:20px;height:120px'><div style='width:20px'>Y Y</div></div><div>Z</div>",
            &[(2, 'Y', 5), (2, 'Y', 35), (3, 'Z', 25)],
        ),
        (
            "<div style='height:150px'></div><div style='display:flex'><div style='width:20px;line-height:120px'>X</div></div><div>Z</div>",
            &[(2, 'X', 50), (3, 'Z', 25)],
        ),
        (
            "<div style='height:150px'></div><div style='display:flex;border-top:5px solid'><div style='width:20px;line-height:120px'>X</div></div><div>Z</div>",
            &[(3, 'X', 50), (4, 'Z', 25)],
        ),
        (
            "<div style='height:94px'></div><div style='display:flex;padding-top:6px'><div style='break-inside:avoid;height:120px;width:20px'>X</div></div><div>Z</div>",
            &[(1, 'X', 11), (2, 'Z', 31)],
        ),
    ]);
}
