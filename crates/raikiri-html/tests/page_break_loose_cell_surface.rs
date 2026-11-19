//! Table cells outside a table, laid out in the anonymous row and table that
//! wrap their run (CSS 2.1 §17.2.1), keep their places when a break inside
//! one of them grows the run, as the cells of a row do, and the boxes after
//! the run follow its growth. Where each glyph falls is what Chromium prints
//! for the same documents on pages 200px wide and 100px tall, with Ahem
//! glyphs of 10px on lines of 20px.

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

/// The page, glyph, left edge and glyph top of every glyph of text in
/// `body`, laid out on the pages and with the font of the Chromium documents.
fn glyphs(body: &str) -> Vec<(u32, char, i32, i32)> {
    let fonts = FontCollectionBuilder::new()
        .font_bytes(
            "Ahem",
            include_bytes!("../../raikiri-dom/tests/data/text-autospace/Ahem.ttf").to_vec(),
        )
        .build()
        .unwrap();
    let resources = RenderResources::new().fonts(fonts);
    let html = format!(
        "<!doctype html><style>@page{{size:200px 100px;margin:0}} body{{margin:0;font:10px/20px Ahem}} table{{border-spacing:0}} td{{padding:0;vertical-align:top}}</style><body>{body}</body>"
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
    let mut glyphs = Vec::new();
    for index in 0..result.page_count() {
        let page = result.page(index).unwrap();
        for run in page.text_runs() {
            // Every Ahem glyph is 10px wide.
            for (offset, glyph) in run.text.chars().enumerate() {
                if !glyph.is_whitespace() {
                    glyphs.push((
                        index,
                        glyph,
                        (run.origin.0 + 10.0 * offset as f32).round() as i32,
                        (run.origin.1 - run.ascent).round() as i32,
                    ));
                }
            }
        }
    }
    glyphs.sort_unstable();
    glyphs
}

#[test]
fn a_forced_break_in_a_loose_cell_leaves_the_cell_beside_in_place() {
    // A forced break grows the first cell; the second keeps its place at the
    // top of the run.
    assert_eq!(
        glyphs(
            r#"<div><div style="display:table-cell"><div style="height:50px">W</div><div style="break-before:page">Y</div></div><div style="display:table-cell"><div style="height:60px">D</div></div></div><div>X</div>"#
        ),
        [
            (0, 'D', 10, 5),
            (0, 'W', 0, 5),
            (1, 'X', 0, 25),
            (1, 'Y', 0, 5),
        ]
    );
}

#[test]
fn a_box_that_avoids_breaks_moving_in_a_loose_cell_leaves_the_cell_beside() {
    // A box that avoids breaks inside it moves to the next page instead.
    assert_eq!(
        glyphs(
            r#"<div><div style="display:table-cell"><div style="height:50px">W</div><div style="height:70px;break-inside:avoid">Y</div></div><div style="display:table-cell"><div style="height:60px">D</div></div></div><div>X</div>"#
        ),
        [
            (0, 'D', 10, 5),
            (0, 'W', 0, 5),
            (1, 'X', 0, 75),
            (1, 'Y', 0, 5),
        ]
    );
}

#[test]
fn a_float_that_avoids_breaks_moving_in_a_loose_cell_leaves_the_cell_beside() {
    // A float that avoids breaks inside it moves to the next page instead.
    assert_eq!(
        glyphs(
            r#"<div><div style="display:table-cell"><div style="height:50px">W</div><div style="float:left;height:70px;break-inside:avoid">Y</div></div><div style="display:table-cell"><div style="height:60px">D</div></div></div><div>X</div>"#
        ),
        [
            (0, 'D', 10, 5),
            (0, 'W', 0, 5),
            (1, 'X', 0, 75),
            (1, 'Y', 0, 5),
        ]
    );
}

#[test]
fn a_forced_break_in_the_second_loose_cell_leaves_the_first_in_place() {
    // The break is in the second cell of the run.
    assert_eq!(
        glyphs(
            r#"<div><div style="display:table-cell"><div style="height:60px">D</div></div><div style="display:table-cell"><div style="height:50px">W</div><div style="break-before:page">Y</div></div></div><div>X</div>"#
        ),
        [
            (0, 'D', 0, 5),
            (0, 'W', 10, 5),
            (1, 'X', 0, 25),
            (1, 'Y', 10, 5),
        ]
    );
}

#[test]
fn a_box_moving_in_the_second_loose_cell_leaves_the_first_in_place() {
    // A box that avoids breaks moves in the second cell of the run.
    assert_eq!(
        glyphs(
            r#"<div><div style="display:table-cell"><div style="height:60px">D</div></div><div style="display:table-cell"><div style="height:50px">W</div><div style="height:70px;break-inside:avoid">Y</div></div></div><div>X</div>"#
        ),
        [
            (0, 'D', 0, 5),
            (0, 'W', 10, 5),
            (1, 'X', 0, 75),
            (1, 'Y', 10, 5),
        ]
    );
}

#[test]
fn boxes_after_a_run_of_loose_cells_follow_its_growth() {
    // A box after the run, inside the same parent, follows the run's growth.
    assert_eq!(
        glyphs(
            r#"<div><div style="display:table-cell"><div style="height:50px">W</div><div style="break-before:page">Y</div></div><div style="display:table-cell"> D</div><div>Z</div></div><div>X</div>"#
        ),
        [
            (0, 'D', 10, 5),
            (0, 'W', 0, 5),
            (1, 'X', 0, 45),
            (1, 'Y', 0, 5),
            (1, 'Z', 0, 25),
        ]
    );
}

#[test]
fn loose_cells_in_the_body_keep_their_places_beside_a_break() {
    // The cells are children of the body.
    assert_eq!(
        glyphs(
            r#"<div style="display:table-cell"><div style="height:50px">W</div><div style="break-before:page">Y</div></div><div style="display:table-cell"><div style="height:60px">D</div></div><div>X</div>"#
        ),
        [
            (0, 'D', 10, 5),
            (0, 'W', 0, 5),
            (1, 'X', 0, 25),
            (1, 'Y', 0, 5),
        ]
    );
}

#[test]
fn forced_breaks_in_two_loose_cells_grow_the_run_once() {
    // Each cell of the run breaks; the run grows as far as the cell that
    // grows most.
    assert_eq!(
        glyphs(
            r#"<div><div style="display:table-cell"><div style="height:50px">W</div><div style="break-before:page">Y</div></div><div style="display:table-cell"><div style="height:30px">D</div><div style="break-before:page">E</div></div></div><div>X</div>"#
        ),
        [
            (0, 'D', 10, 5),
            (0, 'W', 0, 5),
            (1, 'E', 10, 5),
            (1, 'X', 0, 25),
            (1, 'Y', 0, 5),
        ]
    );
}

#[test]
fn a_middle_aligned_loose_cell_beside_a_break_aligns_in_the_first_part() {
    // The cell beside aligns to the middle of the part of the run on the
    // first page.
    assert_eq!(
        glyphs(
            r#"<div><div style="display:table-cell"><div style="height:50px">W</div><div style="break-before:page">Y</div></div><div style="display:table-cell;vertical-align:middle">D</div></div><div>X</div>"#
        ),
        [
            (0, 'D', 10, 45),
            (0, 'W', 0, 5),
            (1, 'X', 0, 25),
            (1, 'Y', 0, 5),
        ]
    );
}

#[test]
fn a_block_between_loose_cells_starts_a_new_run() {
    // A block between the cells ends the first run, so it follows the run's
    // growth, and the cell after it starts a run of its own.
    assert_eq!(
        glyphs(
            r#"<div><div style="display:table-cell"><div style="height:20px">W</div><div style="break-before:page">Y</div></div><div style="display:table-cell"><div style="height:40px">D</div></div><div>M</div><div style="display:table-cell"> E</div></div><div>X</div>"#
        ),
        [
            (0, 'D', 10, 5),
            (0, 'W', 0, 5),
            (1, 'E', 0, 45),
            (1, 'M', 0, 25),
            (1, 'X', 0, 65),
            (1, 'Y', 0, 5),
        ]
    );
}

#[test]
fn a_loose_cell_with_a_set_height_beside_a_break_keeps_its_place() {
    // The cell beside has a height of its own.
    assert_eq!(
        glyphs(
            r#"<div><div style="display:table-cell"><div style="height:50px">W</div><div style="break-before:page">Y</div></div><div style="display:table-cell;height:80px"><div style="height:60px">D</div></div></div><div>X</div>"#
        ),
        [
            (0, 'D', 10, 5),
            (0, 'W', 0, 5),
            (1, 'X', 0, 25),
            (1, 'Y', 0, 5),
        ]
    );
}

#[test]
fn a_loose_cell_beside_a_grown_one_aligns_in_its_fragment_below_earlier_boxes() {
    // The run starts below a box, so the cell beside is aligned within the
    // part of the row before the break, measured from the top of the run.
    assert_eq!(
        glyphs(
            r#"<div style="height:40px">H</div><div style="display:table-cell">W<div style="break-before:page">Y</div></div><div style="display:table-cell;vertical-align:middle">D</div><div>X</div>"#
        ),
        [
            (0, 'D', 10, 65),
            (0, 'H', 0, 5),
            (0, 'W', 0, 45),
            (1, 'X', 0, 25),
            (1, 'Y', 0, 5),
        ]
    );
    assert_eq!(
        glyphs(
            r#"<div style="height:40px">H</div><div style="display:table-cell">W<div style="break-before:page">Y</div></div><div style="display:table-cell;vertical-align:bottom">D</div><div>X</div>"#
        ),
        [
            (0, 'D', 10, 85),
            (0, 'H', 0, 5),
            (0, 'W', 0, 45),
            (1, 'X', 0, 25),
            (1, 'Y', 0, 5),
        ]
    );
}

#[test]
fn a_loose_row_after_a_run_of_loose_cells_follows_the_grown_cell() {
    // A row after the run starts a new anonymous row below it.
    assert_eq!(
        glyphs(
            r#"<div style="display:table-cell">W<div style="break-before:page">Y</div></div><div style="display:table-row"><div style="display:table-cell">R</div></div><div>X</div>"#
        ),
        [
            (0, 'W', 0, 5),
            (1, 'R', 0, 25),
            (1, 'X', 0, 45),
            (1, 'Y', 0, 5),
        ]
    );
}

#[test]
fn a_loose_cell_inside_display_contents_keeps_the_run_together() {
    // The cell beside sits outside the box with `display: contents`.
    assert_eq!(
        glyphs(
            r#"<span style="display:contents"><div style="display:table-cell">W<div style="break-before:page">Y</div></div></span><div style="display:table-cell">D</div><div>X</div>"#
        ),
        [
            (0, 'D', 10, 5),
            (0, 'W', 0, 5),
            (1, 'X', 0, 25),
            (1, 'Y', 0, 5),
        ]
    );
}

#[test]
fn the_cells_of_a_loose_row_stay_beside_a_grown_one() {
    // A row outside a table, alone or inside another table's cell.
    let expected = [
        (0, 'D', 10, 5),
        (0, 'W', 0, 5),
        (1, 'X', 0, 25),
        (1, 'Y', 0, 5),
    ];
    assert_eq!(
        glyphs(
            r#"<div style="display:table-row"><div style="display:table-cell">W<div style="break-before:page">Y</div></div><div style="display:table-cell">D</div></div><div>X</div>"#
        ),
        expected
    );
    assert_eq!(
        glyphs(
            r#"<table><tr><td><div style="display:table-row"><div style="display:table-cell">W<div style="break-before:page">Y</div></div><div style="display:table-cell">D</div></div></td></tr></table><div>X</div>"#
        ),
        expected
    );
}

#[test]
fn a_float_moving_in_a_loose_cell_before_a_forced_break_is_counted_once() {
    // The float grows the cell first; the forced break then adds only the
    // rest of its growth to the boxes after the run.
    assert_eq!(
        glyphs(
            r#"<div style="display:table-cell"><div style="height:70px">W</div><div style="float:left;width:20px;height:40px;break-inside:avoid">F</div><div style="clear:both;break-before:page">Y</div></div><div style="display:table-cell">D</div><div>X</div>"#
        ),
        [
            (0, 'D', 20, 5),
            (0, 'W', 0, 5),
            (1, 'F', 0, 5),
            (2, 'X', 0, 25),
            (2, 'Y', 0, 5),
        ]
    );
    assert_eq!(
        glyphs(
            r#"<div style="display:table-cell"><div style="height:70px">W</div><div style="float:left;width:20px;height:40px;break-inside:avoid">F</div></div><div style="display:table-cell">D</div><div style="display:table-row"><div style="display:table-cell">R</div></div><div>X</div>"#
        ),
        [
            (0, 'D', 20, 5),
            (0, 'W', 0, 5),
            (1, 'F', 0, 5),
            (1, 'R', 0, 45),
            (1, 'X', 0, 65),
        ]
    );
}

#[test]
fn boxes_after_a_loose_run_inside_display_contents_follow_the_grown_cell() {
    // A block or a row after the run inside the same box with
    // `display: contents`, also behind a second such box, moves down with
    // the growth, as do the boxes after the wrapper.
    for (html, after) in [
        (
            r#"<span style="display:contents"><div style="display:table-cell">W<div style="break-before:page">Y</div></div><div style="display:table-cell">D</div><div>T</div></span><div>X</div>"#,
            'T',
        ),
        (
            r#"<span style="display:contents"><div style="display:table-cell">W<div style="break-before:page">Y</div></div><div style="display:table-cell">D</div><div style="display:table-row"><div style="display:table-cell">R</div></div></span><div>X</div>"#,
            'R',
        ),
        (
            r#"<span style="display:contents"><span style="display:contents"><div style="display:table-cell">W<div style="break-before:page">Y</div></div></span><div style="display:table-cell">D</div><div>T</div></span><div>X</div>"#,
            'T',
        ),
    ] {
        let mut expected = vec![
            (0, 'D', 10, 5),
            (0, 'W', 0, 5),
            (1, after, 0, 25),
            (1, 'X', 0, 45),
            (1, 'Y', 0, 5),
        ];
        expected.sort_unstable();
        assert_eq!(glyphs(html), expected, "{html}");
    }
}
