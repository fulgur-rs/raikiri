//! A box placed out of the flow is not part of the flow it sits in, so it
//! does not keep a container on its page. When the first line of the
//! container's content crosses the page end and starts the next page, the
//! container starts that page with it, and the absolutely positioned boxes it
//! holds go along: one placed against the container's top or bottom, and one
//! at its static position. Where each line falls is what Chromium prints for
//! the same documents on pages 100px tall, with Ahem lines of 20px and a line
//! height of 30px; the positioned box `C` is 4px tall.

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

/// A document body and the page, glyph and top of each line.
type Row = (&'static str, &'static [(u32, char, i32)]);

fn assert_rows(rows: &[Row]) {
    for &(body, expected) in rows {
        assert_eq!(glyph_tops(body), expected, "{body}");
    }
}

#[test]
fn a_container_moved_with_its_first_line_takes_its_positioned_boxes_along() {
    // A relative block, column flex or grid container, with the positioned
    // box before or after its lines, placed by `top: 0`, `top: 10px` or
    // `bottom: 0`.
    assert_rows(&[
        (
            "<div style='height:94px'></div><div style='display:flex;flex-direction:column;position:relative'><div style='position:absolute;top:0;left:40px;font-size:4px;line-height:4px'>C</div><p style='margin:0'>A</p><p style='margin:0'>B</p></div>",
            &[(1, 'A', 5), (1, 'B', 35), (1, 'C', 0)],
        ),
        (
            "<div style='height:94px'></div><div style='position:relative'><div style='position:absolute;top:0;left:40px;font-size:4px;line-height:4px'>C</div><p style='margin:0'>A</p><p style='margin:0'>B</p></div>",
            &[(1, 'A', 5), (1, 'B', 35), (1, 'C', 0)],
        ),
        (
            "<div style='height:94px'></div><div style='display:grid;position:relative'><div style='position:absolute;top:0;left:40px;font-size:4px;line-height:4px'>C</div><p style='margin:0'>A</p><p style='margin:0'>B</p></div>",
            &[(1, 'A', 5), (1, 'B', 35), (1, 'C', 0)],
        ),
        (
            "<div style='height:94px'></div><div style='display:flex;flex-direction:column;position:relative'><div style='position:absolute;top:10px;left:40px;font-size:4px;line-height:4px'>C</div><p style='margin:0'>A</p><p style='margin:0'>B</p></div>",
            &[(1, 'A', 5), (1, 'B', 35), (1, 'C', 10)],
        ),
        (
            "<div style='height:94px'></div><div style='position:relative'><div style='position:absolute;top:10px;left:40px;font-size:4px;line-height:4px'>C</div><p style='margin:0'>A</p><p style='margin:0'>B</p></div>",
            &[(1, 'A', 5), (1, 'B', 35), (1, 'C', 10)],
        ),
        (
            "<div style='height:94px'></div><div style='position:relative'><div><div style='position:absolute;top:0;left:40px;font-size:4px;line-height:4px'>C</div><p style='margin:0'>A</p><p style='margin:0'>B</p></div></div>",
            &[(1, 'A', 5), (1, 'B', 35), (1, 'C', 0)],
        ),
        (
            "<div style='height:94px'></div><div style='display:flex;flex-direction:column;position:relative'><p style='margin:0'>A</p><p style='margin:0'>B</p><div style='position:absolute;top:0;left:40px;font-size:4px;line-height:4px'>C</div></div>",
            &[(1, 'A', 5), (1, 'B', 35), (1, 'C', 0)],
        ),
        (
            "<div style='height:94px'></div><div style='display:flex;flex-direction:column;position:relative'><div style='position:absolute;bottom:0;left:40px;font-size:4px;line-height:4px'>C</div><p style='margin:0'>A</p><p style='margin:0'>B</p></div>",
            &[(1, 'A', 5), (1, 'B', 35), (1, 'C', 56)],
        ),
    ]);
}

#[test]
fn positioned_boxes_do_not_keep_a_container_on_its_page() {
    // A box at its static position in a block or column flex container, and
    // in a wrapper inside a relative block, moves with the first line. A box
    // placed against the page stays, and so does a container whose first
    // line fits its page.
    assert_rows(&[
        (
            "<div style='height:94px'></div><div><div style='position:absolute;top:0;left:40px;font-size:4px;line-height:4px'>C</div><p style='margin:0'>A</p><p style='margin:0'>B</p></div>",
            &[(0, 'C', 0), (1, 'A', 5), (1, 'B', 35)],
        ),
        (
            "<div style='height:94px'></div><div><div style='position:absolute;left:40px;font-size:4px;line-height:4px'>C</div><p style='margin:0'>A</p><p style='margin:0'>B</p></div>",
            &[(1, 'A', 5), (1, 'B', 35), (1, 'C', 0)],
        ),
        (
            "<div style='height:94px'></div><div style='position:relative'><div style='position:absolute;left:40px;font-size:4px;line-height:4px'>C</div><p style='margin:0'>A</p><p style='margin:0'>B</p></div>",
            &[(1, 'A', 5), (1, 'B', 35), (1, 'C', 0)],
        ),
        (
            "<div style='height:64px'></div><div style='position:relative'><div style='position:absolute;top:0;left:40px;font-size:4px;line-height:4px'>C</div><p style='margin:0'>A</p><p style='margin:0'>B</p></div>",
            &[(0, 'A', 69), (0, 'C', 64), (1, 'B', 5)],
        ),
        (
            "<div style='height:94px'></div><div style='position:relative'><div><div style='position:absolute;top:0;left:40px;font-size:4px;line-height:4px'>C</div><p style='margin:0'>A</p><p style='margin:0'>B</p></div></div>",
            &[(1, 'A', 5), (1, 'B', 35), (1, 'C', 0)],
        ),
        (
            "<div style='height:94px'></div><div style='position:relative'><div><p style='margin:0'>A</p><p style='margin:0'>B</p></div><div style='position:absolute;top:0;left:40px;font-size:4px;line-height:4px'>C</div></div>",
            &[(1, 'A', 5), (1, 'B', 35), (1, 'C', 0)],
        ),
        (
            "<div style='height:94px'></div><div style='display:flex;flex-direction:column'><div style='position:absolute;left:40px;font-size:4px;line-height:4px'>C</div><p style='margin:0'>A</p><p style='margin:0'>B</p></div>",
            &[(1, 'A', 5), (1, 'B', 35), (1, 'C', 0)],
        ),
    ]);
}
