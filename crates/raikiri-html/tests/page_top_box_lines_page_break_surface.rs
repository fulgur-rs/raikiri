//! A paragraph whose box starts a page with padding or border above it breaks
//! between its lines when too few of them fit for its orphans: moving the box
//! to the next page would gain it no room. Empty boxes at the very start of
//! the page do not count as content before the box, and neither does a flex or
//! grid container that holds the box as its only item with no edges above it.
//! Where each line falls is what Chromium prints for the same documents on
//! pages 100px tall, with Ahem lines of 20px and a line height of 30px; a `Z`
//! block after the box shows where the box ends.

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

/// A document and the page, glyph and top of each of its lines.
type Row = (&'static str, &'static [(u32, char, i32)]);

const PARAGRAPH: &str = "<p style='margin:0;orphans:3;widows:1'>X X X</p>";
const AFTER: &str = "<div style='height:30px'>Z</div>";

fn assert_rows(rows: &[Row]) {
    for &(before, expected) in rows {
        let body = before.replace("{p}", PARAGRAPH) + AFTER;
        assert_eq!(glyph_tops(&body), expected, "{body}");
    }
}

#[test]
fn a_box_starting_the_page_breaks_between_its_lines() {
    assert_rows(&[
        (
            "<div style='height:0px'></div><div style='width:20px;padding:10px 0;border-top:5px solid'>{p}</div>",
            &[(0, 'X', 20), (0, 'X', 50), (1, 'X', 5), (1, 'Z', 45)],
        ),
        (
            "<div style='height:0px'></div><div style='width:20px;border-top:15px solid'>{p}</div>",
            &[(0, 'X', 20), (0, 'X', 50), (1, 'X', 5), (1, 'Z', 35)],
        ),
        (
            "<div></div><div style='width:20px;padding-top:15px'>{p}</div>",
            &[(0, 'X', 20), (0, 'X', 50), (1, 'X', 5), (1, 'Z', 35)],
        ),
        (
            "<div style='display:contents'></div><div style='width:20px;padding-top:15px'>{p}</div>",
            &[(0, 'X', 20), (0, 'X', 50), (1, 'X', 5), (1, 'Z', 35)],
        ),
        // A page the flow reaches, and one a forced break opens.
        (
            "<div style='height:100px'></div><div style='height:0'></div><div style='width:20px;padding-top:15px'>{p}</div>",
            &[(1, 'X', 20), (1, 'X', 50), (2, 'X', 5), (2, 'Z', 35)],
        ),
        (
            "<div style='height:20px;break-after:page'></div><div></div><div style='width:20px;padding-top:15px'>{p}</div>",
            &[(1, 'X', 20), (1, 'X', 50), (2, 'X', 5), (2, 'Z', 35)],
        ),
        // Avoiding a break inside the box does not move it either.
        (
            "<div style='height:0'></div><div style='width:20px;padding-top:15px;break-inside:avoid'>{p}</div>",
            &[(0, 'X', 20), (0, 'X', 50), (1, 'X', 5), (1, 'Z', 35)],
        ),
        // Content before the box: it moves to keep the orphans.
        (
            "<div style='height:10px'></div><div style='width:20px;padding:10px 0;border-top:5px solid'>{p}</div>",
            &[(1, 'X', 20), (1, 'X', 50), (2, 'X', 5), (2, 'Z', 45)],
        ),
    ]);
}

#[test]
fn the_only_item_starting_the_page_breaks_between_its_lines() {
    assert_rows(&[
        (
            "<div style='width:20px;display:grid'><div style='padding-top:15px'>{p}</div></div>",
            &[(0, 'X', 20), (0, 'X', 50), (1, 'X', 5), (1, 'Z', 35)],
        ),
        (
            "<div style='width:20px;display:flex;flex-direction:column'><div style='padding-top:15px'>{p}</div></div>",
            &[(0, 'X', 20), (0, 'X', 50), (1, 'X', 5), (1, 'Z', 35)],
        ),
        (
            "<div style='width:20px;display:flex'><div style='padding-top:15px'>{p}</div></div>",
            &[(0, 'X', 20), (0, 'X', 50), (1, 'X', 5), (1, 'Z', 35)],
        ),
        // Boxes out of the flow of the grid are no items.
        (
            "<div style='width:20px;display:grid;position:relative'><div style='position:absolute'></div><div style='padding-top:15px'>{p}</div></div>",
            &[(0, 'X', 20), (0, 'X', 50), (1, 'X', 5), (1, 'Z', 35)],
        ),
        (
            "<div style='width:20px;display:grid'><div style='display:none'>Q</div><div style='padding-top:15px'>{p}</div></div>",
            &[(0, 'X', 20), (0, 'X', 50), (1, 'X', 5), (1, 'Z', 35)],
        ),
        // The padding of the grid is content the page breaks after: the item
        // moves.
        (
            "<div style='width:20px;display:grid;padding-top:10px'><div style='padding-top:15px'>{p}</div></div>",
            &[(1, 'X', 20), (1, 'X', 50), (2, 'X', 5), (2, 'Z', 35)],
        ),
    ]);
}
