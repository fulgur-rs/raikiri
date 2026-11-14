//! The lines of a paragraph inside a float around the end of a page. They
//! break across pages as lines in a block do, and the float grows with the
//! lines it moves. The float's growth moves only what clears it or what holds
//! it in its own formatting context; the in-flow boxes beside it stay. Where
//! each line falls is what Chromium prints for the same documents on pages
//! 100px tall, with Ahem lines of 20px and a line height of 30px.

use raikiri_html::{
    DocumentLayout, FontCollectionBuilder, FragmentKind, LayoutOptions, LayoutStatus,
    RenderResources, layout, parse_html_with_resources,
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

/// The page, glyph and glyph top of every line of text in `body`.
fn glyph_tops(body: &str) -> Vec<(u32, char, i32)> {
    let result = lay_out(body);
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

#[test]
fn a_float_moves_apart_from_its_wrapper_and_its_later_paragraphs_follow_its_lines() {
    assert_rows(&[
        (
            "<div style='height:60px'></div><div><div style='float:left;width:20px'>X X X</div></div><div style='height:30px'>Z</div>",
            &[(0, 'Z', 65), (1, 'X', 5), (1, 'X', 35), (1, 'X', 65)],
        ),
        (
            "<div style='height:30px'></div><div style='float:left;width:20px'><p style='margin:0'>X X X</p><p style='margin:0'>Y Y Y</p><p style='margin:0'>W W W</p></div>",
            &[
                (0, 'X', 35),
                (0, 'X', 65),
                (1, 'X', 5),
                (1, 'Y', 35),
                (1, 'Y', 65),
                (2, 'W', 35),
                (2, 'W', 65),
                (2, 'Y', 5),
                (3, 'W', 5),
            ],
        ),
        (
            "<div style='height:30px'></div><div style='float:left;width:20px'><p style='margin:0;orphans:1;widows:1'>X X X</p><p style='margin:0;orphans:1;widows:1'>Y Y Y Y</p></div>",
            &[
                (0, 'X', 35),
                (0, 'X', 65),
                (1, 'X', 5),
                (1, 'Y', 35),
                (1, 'Y', 65),
                (2, 'Y', 5),
                (2, 'Y', 35),
            ],
        ),
    ]);
}

#[test]
fn a_box_in_a_float_that_clears_a_float_grown_beside_it_stays_below_it() {
    assert_rows(&[(
        "<div style='height:20px'></div><div style='float:left;width:60px'><div style='float:left;width:20px'>X X X</div><div style='clear:both'>Z</div></div>",
        &[(0, 'X', 25), (0, 'X', 55), (1, 'X', 5), (1, 'Z', 35)],
    )]);
}

/// The top and height of each 60px wide box on page `index` of `body`.
fn wide_boxes(body: &str, index: u32) -> Vec<(f32, f32)> {
    let document = lay_out(body);
    let page = document.page(index).unwrap();
    page.fragments()
        .filter(|fragment| fragment.kind() == FragmentKind::Box)
        .map(|fragment| fragment.rect())
        .filter(|rect| rect.width == 60.0)
        .map(|rect| (rect.y, rect.height))
        .collect()
}

#[test]
fn a_box_without_text_in_a_float_that_clears_a_float_grown_beside_it_stays_below_it() {
    // The 60px float holds the 20px float, whose last line moves to the
    // second page, and a 20px box below it that clears it, then a 10px box.
    let body = "<div style='height:20px'></div><div style='float:left;width:60px'><div style='float:left;width:20px'>X X X</div> <div style='clear:left;height:20px'></div><div style='height:10px'></div></div>";
    assert_eq!(
        wide_boxes(body, 1),
        [(0.0, 60.0), (30.0, 20.0), (50.0, 10.0)]
    );
    // An empty box between them stays beside the float.
    let body = "<div style='height:20px'></div><div style='float:left;width:60px'><div style='float:left;width:20px'>X X X</div><div></div><div style='clear:left;height:20px'></div><div style='height:10px'></div></div>";
    assert_eq!(wide_boxes(body, 0), [(20.0, 80.0), (20.0, 0.0)]);
    assert_eq!(
        wide_boxes(body, 1),
        [(0.0, 60.0), (30.0, 20.0), (50.0, 10.0)]
    );
    // A relative offset moves the clearing box from there, and not the boxes
    // after it.
    let body = "<div style='height:20px'></div><div style='float:left;width:60px'><div style='float:left;width:20px'>X X X</div> <div style='clear:left;height:20px;position:relative;top:10px'></div><div style='height:10px'></div></div>";
    assert_eq!(
        wide_boxes(body, 1),
        [(0.0, 60.0), (40.0, 20.0), (50.0, 10.0)]
    );
    // A box that clears only the other side stays beside the float.
    let body = "<div style='height:20px'></div><div style='float:left;width:60px'><div style='float:right;width:20px'>X X X</div><div style='clear:left;height:20px'></div></div>";
    assert_eq!(wide_boxes(body, 0), [(20.0, 80.0), (20.0, 20.0)]);
    assert_eq!(wide_boxes(body, 1), [(0.0, 30.0)]);
}

#[test]
fn a_float_below_a_float_grown_inside_another_float_moves_down_with_it() {
    assert_rows(&[
        // Too wide to sit beside the grown float.
        (
            "<div style='height:20px'></div><div style='float:left;width:60px'><div style='float:left;width:20px'>X X X</div><div style='float:left;width:60px'>Y</div></div>",
            &[(0, 'X', 25), (0, 'X', 55), (1, 'X', 5), (1, 'Y', 35)],
        ),
        // The flow after the outer float does not follow the moved float.
        (
            "<div style='height:20px'></div><div style='float:left;width:60px'><div style='float:left;width:20px'>X X X</div><div style='float:left;width:60px'>Y</div></div><div>W</div>",
            &[
                (0, 'W', 25),
                (0, 'X', 25),
                (0, 'X', 55),
                (1, 'X', 5),
                (1, 'Y', 35),
            ],
        ),
        // Beside the grown float, it stays.
        (
            "<div style='height:20px'></div><div style='float:left;width:60px'><div style='float:left;width:20px'>X X X</div><div style='float:left;width:20px'>Y</div></div>",
            &[(0, 'X', 25), (0, 'X', 55), (0, 'Y', 25), (1, 'X', 5)],
        ),
        // A box beside the floats keeps its place.
        (
            "<div style='height:20px'></div><div style='float:left;width:60px'><div style='float:left;width:20px'>X X X</div><div style='float:left;width:60px'>Y</div><div>Z</div></div>",
            &[
                (0, 'X', 25),
                (0, 'X', 55),
                (0, 'Z', 25),
                (1, 'X', 5),
                (1, 'Y', 35),
            ],
        ),
        // A float that clears the grown float.
        (
            "<div style='height:20px'></div><div style='float:left;width:60px'><div style='float:left;width:20px'>X X X</div><div style='float:right;width:20px;clear:left'>Y</div></div>",
            &[(0, 'X', 25), (0, 'X', 55), (1, 'X', 5), (1, 'Y', 35)],
        ),
        // Text beside the grown float, and a box out of the flow, keep their
        // places.
        (
            "<div style='height:20px'></div><div style='float:left;width:60px'><div style='float:left;width:20px'>X X X</div>Y</div>",
            &[(0, 'X', 25), (0, 'X', 55), (0, 'Y', 25), (1, 'X', 5)],
        ),
        (
            "<div style='height:20px'></div><div style='float:left;width:60px'><div style='float:left;width:20px'>X X X</div><div style='position:absolute;width:60px;clear:left'>Y</div></div>",
            &[(0, 'X', 25), (0, 'X', 55), (0, 'Y', 25), (1, 'X', 5)],
        ),
        // A hidden element that would clear the float generates no box.
        (
            "<div style='height:20px'></div><div style='float:left;width:60px'><div style='float:left;width:20px'>X X X</div><div style='display:none;clear:both'></div><div>Z</div></div>",
            &[(0, 'X', 25), (0, 'X', 55), (0, 'Z', 25), (1, 'X', 5)],
        ),
        // A box beside the grown float keeps its place, and a float or a
        // box that clears it after that box still moves below it.
        (
            "<div style='height:20px'></div><div style='float:left;width:60px'><div style='float:left;width:20px'>X X X</div><div>Z</div><div style='float:left;width:60px'>Y</div></div>",
            &[
                (0, 'X', 25),
                (0, 'X', 55),
                (0, 'Z', 25),
                (1, 'X', 5),
                (1, 'Y', 35),
            ],
        ),
        (
            "<div style='height:20px'></div><div style='float:left;width:60px'><div style='float:left;width:20px'>X X X</div><div>Z</div><div style='clear:left'>W</div></div>",
            &[
                (0, 'X', 25),
                (0, 'X', 55),
                (0, 'Z', 25),
                (1, 'W', 35),
                (1, 'X', 5),
            ],
        ),
        // A box that clears both floats follows the moved one.
        (
            "<div style='height:20px'></div><div style='float:left;width:60px'><div style='float:left;width:20px'>X X X</div><div style='float:left;width:60px'>Y</div><div style='clear:both'>Z</div></div>",
            &[
                (0, 'X', 25),
                (0, 'X', 55),
                (1, 'X', 5),
                (1, 'Y', 35),
                (1, 'Z', 65),
            ],
        ),
    ]);
}
