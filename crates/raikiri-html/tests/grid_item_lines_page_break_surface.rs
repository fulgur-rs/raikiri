//! The only item of a grid whose rows size to their content breaks between
//! the lines of its paragraphs at a page end as a block does: it does not move
//! to the next page whole because it would fit there. An item whose lines do
//! not break that way, such as one with size containment, still moves whole.
//! Where each line falls is what
//! Chromium prints for the same documents on pages 100px tall, with Ahem
//! lines of 20px and a line height of 30px; a `Z` block after the grid shows
//! where the grid ends.

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

#[test]
fn the_only_grid_item_breaks_between_its_lines() {
    let rows: &[Row] = &[
        (
            "<div style='height:30px'></div><div style='width:20px;display:grid'><p style='margin:0;orphans:1;widows:1'>X X X</p></div><div style='height:30px'>Z</div>",
            &[(0, 'X', 35), (0, 'X', 65), (1, 'X', 5), (1, 'Z', 35)],
        ),
        // The paragraph sits in a block that is the item.
        (
            "<div style='height:30px'></div><div style='width:20px;display:grid'><div><p style='margin:0;orphans:1;widows:1'>X X X</p></div></div><div style='height:30px'>Z</div>",
            &[(0, 'X', 35), (0, 'X', 65), (1, 'X', 5), (1, 'Z', 35)],
        ),
        // The item clips at its height: the line it pushes past the clip is
        // hidden.
        (
            "<div style='height:30px'></div><div style='width:20px;display:grid'><div style='height:90px;overflow:hidden'><p style='margin:0;orphans:1;widows:1'>X X X</p></div></div><div style='height:30px'>Z</div>",
            &[(0, 'X', 35), (0, 'X', 65), (1, 'X', 5), (1, 'Z', 25)],
        ),
        // Size containment keeps the lines where they are: the item moves.
        (
            "<div style='height:30px'></div><div style='width:20px;display:grid'><div style='height:90px;contain:size'><p style='margin:0;orphans:1;widows:1'>X X X</p></div></div><div style='height:30px'>Z</div>",
            &[(1, 'X', 5), (1, 'X', 35), (1, 'X', 65), (2, 'Z', 5)],
        ),
        // Too few lines fit for the orphans: the lines move together.
        (
            "<div style='height:50px'></div><div style='width:20px;display:grid'><p style='margin:0;orphans:2;widows:2'>X X X</p></div><div style='height:30px'>Z</div>",
            &[(1, 'X', 5), (1, 'X', 35), (1, 'X', 65), (2, 'Z', 5)],
        ),
    ];
    for &(body, expected) in rows {
        assert_eq!(glyph_tops(body), expected, "{body}");
    }
}
