//! The lines of a paragraph inside a float around the end of a page. They
//! break across pages as lines in a block do, and the float grows with the
//! lines it moves. The float's growth moves only what clears it or what holds
//! it in its own formatting context; the in-flow boxes beside it stay. Where
//! each line falls is what Chromium prints for the same documents on pages
//! 100px tall, with Ahem lines of 20px and a line height of 30px.

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

fn assert_rows(rows: &[Row]) {
    for &(body, expected) in rows {
        assert_eq!(glyph_tops(body), expected, "{body}");
    }
}

#[test]
fn the_lines_inside_a_float_break_across_pages() {
    assert_rows(&[
        (
            "<div style='height:30px'></div><div style='width:60px'><div><div style='float:left;width:20px'>X X X</div></div></div><div style='height:30px'>Z</div>",
            &[(0, 'X', 35), (0, 'X', 65), (0, 'Z', 35), (1, 'X', 5)],
        ),
        (
            "<div style='height:30px'></div><div style='float:left;width:20px'>X X X</div><div style='height:30px'>Z</div>",
            &[(0, 'X', 35), (0, 'X', 65), (0, 'Z', 35), (1, 'X', 5)],
        ),
        (
            "<div style='height:30px'></div><div style='float:left;width:20px'><div>X X X</div></div><div style='height:30px'>Z</div>",
            &[(0, 'X', 35), (0, 'X', 65), (0, 'Z', 35), (1, 'X', 5)],
        ),
        (
            "<div style='height:30px'></div><div style='width:60px'><div style='float:left;width:20px'>X X X</div></div><div style='height:30px'>Z</div>",
            &[(0, 'X', 35), (0, 'X', 65), (0, 'Z', 35), (1, 'X', 5)],
        ),
        (
            "<div style='height:30px'></div><div style='width:60px'><div style='float:left;width:20px'>X X X X X X</div></div>",
            &[
                (0, 'X', 35),
                (0, 'X', 65),
                (1, 'X', 5),
                (1, 'X', 35),
                (2, 'X', 5),
                (2, 'X', 35),
            ],
        ),
        (
            "<div style='height:30px'></div><div style='width:60px'><div style='float:left;width:20px;orphans:1;widows:1'>X X X</div></div>",
            &[(0, 'X', 35), (0, 'X', 65), (1, 'X', 5)],
        ),
        (
            "<div style='height:30px'></div><div style='float:left;width:20px'>X X X X X X X X</div><div style='clear:both'>Z</div>",
            &[
                (0, 'X', 35),
                (0, 'X', 65),
                (1, 'X', 5),
                (1, 'X', 35),
                (1, 'X', 65),
                (2, 'X', 5),
                (2, 'X', 35),
                (2, 'X', 65),
                (3, 'Z', 5),
            ],
        ),
    ]);
}

#[test]
fn a_float_that_grows_moves_only_what_clears_it() {
    assert_rows(&[
        (
            "<div style='height:30px'></div><div style='float:left;width:20px'>X X X</div><div style='clear:both'>Z</div>",
            &[(0, 'X', 35), (0, 'X', 65), (1, 'X', 5), (1, 'Z', 35)],
        ),
        (
            "<div style='height:30px'></div><div style='width:60px'><div style='float:left;width:20px'>X X X</div></div><div style='clear:both'>Z</div>",
            &[(0, 'X', 35), (0, 'X', 65), (1, 'X', 5), (1, 'Z', 35)],
        ),
        (
            "<div style='height:30px'></div><div style='float:left;width:20px'>X X X</div><div style='float:left;width:20px'>Y</div><div style='clear:left'>Z</div>",
            &[
                (0, 'X', 35),
                (0, 'X', 65),
                (0, 'Y', 35),
                (1, 'X', 5),
                (1, 'Z', 35),
            ],
        ),
        (
            "<div style='height:30px'></div><div style='float:left;width:20px;padding-bottom:5px;border-bottom:3px solid'>X X X</div><div style='clear:both'>Z</div>",
            &[(0, 'X', 35), (0, 'X', 65), (1, 'X', 5), (1, 'Z', 43)],
        ),
        (
            "<div style='height:30px'></div><div style='float:left;width:20px;margin-bottom:10px'>X X X</div><div style='clear:both'>Z</div>",
            &[(0, 'X', 35), (0, 'X', 65), (1, 'X', 5), (1, 'Z', 45)],
        ),
        (
            "<div style='height:60px'></div><div style='float:left;width:20px'>X X X</div><div style='clear:both'>Z</div>",
            &[(1, 'X', 5), (1, 'X', 35), (1, 'X', 65), (2, 'Z', 5)],
        ),
        (
            "<div style='height:30px'></div><div style='float:left;width:20px;orphans:3'>X X X X</div><div style='clear:both'>Z</div>",
            &[
                (1, 'X', 5),
                (1, 'X', 35),
                (1, 'X', 65),
                (2, 'X', 5),
                (2, 'Z', 35),
            ],
        ),
        (
            "<div style='height:30px'></div><div style='width:60px'><div style='float:left;width:20px'>X X X</div><div style='float:left;width:20px;clear:left'>Y</div></div>",
            &[(0, 'X', 35), (0, 'X', 65), (1, 'X', 5), (1, 'Y', 35)],
        ),
        (
            "<div style='height:30px'></div><div style='width:60px;display:flow-root'><div style='float:left;width:20px'>X X X</div><div style='float:left;width:20px'>Y Y Y Y</div></div><div style='height:30px'>Z</div>",
            &[
                (0, 'X', 35),
                (0, 'X', 65),
                (0, 'Y', 35),
                (0, 'Y', 65),
                (1, 'X', 5),
                (1, 'Y', 5),
                (1, 'Y', 35),
                (1, 'Z', 65),
            ],
        ),
    ]);
}

#[test]
fn a_formatting_context_grows_around_the_float() {
    assert_rows(&[
        (
            "<div style='height:30px'></div><div style='width:60px;display:flow-root'><div style='float:left;width:20px'>X X X</div></div><div style='height:30px'>Z</div>",
            &[(0, 'X', 35), (0, 'X', 65), (1, 'X', 5), (1, 'Z', 35)],
        ),
        (
            "<div style='height:30px'></div><div style='display:flow-root;padding-bottom:10px'><div style='float:left;width:20px'>X X X</div>Y</div><div>Z</div>",
            &[
                (0, 'X', 35),
                (0, 'X', 65),
                (0, 'Y', 35),
                (1, 'X', 5),
                (1, 'Z', 45),
            ],
        ),
        (
            "<div style='height:30px'></div><div style='width:60px;display:grid'><div><div style='float:left;width:20px'>X X X</div></div></div><div style='height:30px'>Z</div>",
            &[(0, 'X', 35), (0, 'X', 65), (1, 'X', 5), (1, 'Z', 35)],
        ),
    ]);
}

#[test]
fn a_float_in_a_paragraph_breaks_its_own_lines() {
    assert_rows(&[
        (
            "<div style='height:30px'></div><div style='display:flow-root;width:60px'><div style='float:left;width:20px'>X X X</div>Y</div><div>Z</div>",
            &[
                (0, 'X', 35),
                (0, 'X', 65),
                (0, 'Y', 35),
                (1, 'X', 5),
                (1, 'Z', 35),
            ],
        ),
        (
            "<div style='height:30px'></div><div style='width:60px'><div style='float:left;width:20px'>X X X</div>Y</div><div>Z</div>",
            &[
                (0, 'X', 35),
                (0, 'X', 65),
                (0, 'Y', 35),
                (0, 'Z', 65),
                (1, 'X', 5),
            ],
        ),
        (
            "<div style='height:30px'></div><p style='margin:0;width:60px'>A <span style='float:left;width:20px'>X X X</span></p><div style='height:30px'>Z</div>",
            &[
                (0, 'A', 35),
                (0, 'X', 35),
                (0, 'X', 65),
                (0, 'Z', 65),
                (1, 'X', 5),
            ],
        ),
    ]);
}
