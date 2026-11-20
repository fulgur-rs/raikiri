//! Where a run of boxes tied by an avoided break starts when its first box
//! holds boxes of its own: Chromium takes the last break inside that box
//! rather than the one before it. Pages are 200x100 with no margin, and text
//! is Ahem 20px on 20px lines; `S` and `X` are other text. Expected values are
//! Chromium's print layout: the page and top of `#t` (in the first box) and
//! `#z` (the box that avoids the break before it).

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

const AHEM: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/data/text-autospace/Ahem.ttf"
));

/// The page index and the top on that page of the first fragment of each of
/// the elements `#t` and `#z`.
fn places(body: &str) -> [(usize, f32); 2] {
    let html = format!(
        "<!DOCTYPE html><style>@page{{size:200px 100px;margin:0}}\
         body{{margin:0;font:20px/20px Ahem}}</style>{body}"
    );
    let fonts = FontCollectionBuilder::new()
        .font_bytes("Ahem", AHEM.to_vec())
        .build()
        .expect("fonts");
    let resources = RenderResources::new().fonts(fonts);
    let doc = parse_html_with_resources(html.as_bytes(), &resources).expect("parse");
    let status = layout(
        &doc,
        PageDefaults::default(),
        LayoutConfig::default(),
        LayoutOptions::new().resources(&resources),
    )
    .expect("layout");
    let LayoutStatus::Completed(result) = status else {
        panic!("expected a complete layout");
    };
    let mut found = [None, None];
    for (index, page) in result.pages().enumerate() {
        for fragment in page.fragments() {
            let slot = match page.dom().attr(fragment.node(), "id") {
                Some("t") => 0,
                Some("z") => 1,
                _ => continue,
            };
            found[slot].get_or_insert((index, fragment.rect().y));
        }
    }
    found.map(|place| place.unwrap_or_else(|| panic!("no fragment for #t or #z in {body}")))
}

fn check(cases: &[(&str, [(usize, f32); 2])]) {
    let failures: Vec<String> = cases
        .iter()
        .filter_map(|&(body, expected)| {
            let got = places(body);
            (got != expected).then(|| format!("{body}: got {got:?}, expected {expected:?}"))
        })
        .collect();
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn a_run_starts_at_the_last_break_between_the_boxes_of_its_first_box() {
    // The first box of a run tied by an avoided break holds boxes of its own: the
    // page breaks before the last of them that nothing ties to the one before
    // it, and only that part moves with the run.
    check(&[
        (
            "<div style=\"height:50px\">S</div><div><div><span id=t>L</span></div><div>X</div></div><div style=\"break-before:avoid\"><span id=z>Z</span></div>",
            [(0, 50.0), (1, 20.0)],
        ),
        (
            "<div style=\"height:30px\">S</div><div><div>K</div><div><span id=t>L</span></div><div>X</div></div><div style=\"break-before:avoid\"><span id=z>Z</span></div>",
            [(0, 50.0), (1, 20.0)],
        ),
        (
            "<div style=\"height:50px\">S</div><div><div><div><span id=t>L</span></div><div>X</div></div></div><div style=\"break-before:avoid\"><span id=z>Z</span></div>",
            [(0, 50.0), (1, 20.0)],
        ),
        (
            "<div style=\"height:50px\">S</div><div style=\"padding-bottom:10px\"><div><span id=t>L</span></div><div>X</div></div><div style=\"break-before:avoid\"><span id=z>Z</span></div>",
            [(0, 50.0), (1, 30.0)],
        ),
        (
            "<div style=\"height:50px\">S</div><div style=\"break-after:avoid\"><div><span id=t>L</span></div><div>X</div></div><div><span id=z>Z</span></div>",
            [(0, 50.0), (1, 20.0)],
        ),
        (
            "<div style=\"height:50px\">S</div><div><div style=\"display:contents\"><div><span id=t>L</span></div><div>X</div></div></div><div style=\"break-before:avoid\"><span id=z>Z</span></div>",
            [(0, 50.0), (1, 20.0)],
        ),
        (
            "<div style=\"height:50px\">S</div><div><div><span id=t>L</span></div><div>X</div><div style=\"float:left;width:10px;height:10px\"></div></div><div style=\"break-before:avoid\"><span id=z>Z</span></div>",
            [(0, 50.0), (1, 20.0)],
        ),
        (
            "<div style=\"height:50px\">S</div><div><div><span id=t>L</span></div><div style=\"margin-bottom:10px\">X</div></div><div style=\"break-before:avoid\"><span id=z>Z</span></div>",
            [(0, 50.0), (1, 30.0)],
        ),
        (
            "<div style=\"height:50px\">S</div><div><div><span id=t>L</span></div><div style=\"padding-top:5px\">X</div></div><div style=\"break-before:avoid\"><span id=z>Z</span></div>",
            [(0, 50.0), (1, 25.0)],
        ),
        (
            "<div style=\"height:50px\">S</div><div><div><span id=t>L</span></div><div>X</div><div></div></div><div style=\"break-before:avoid\"><span id=z>Z</span></div>",
            [(0, 50.0), (1, 0.0)],
        ),
        (
            "<div style=\"height:50px\">S</div><div><div><span id=t>L</span></div><div>X</div></div><table style=\"break-before:avoid;border-spacing:0\"><tr><td style=\"padding:0\"><span id=z>Z</span></td></tr></table>",
            [(0, 50.0), (1, 20.0)],
        ),
    ]);
}

#[test]
fn boxes_tied_inside_the_first_box_move_with_the_run() {
    // Avoided breaks inside the first box, or a chain of them, move the boxes
    // they tie along.
    check(&[
        (
            "<div style=\"height:30px\">S</div><div><div>K</div><div style=\"break-after:avoid\"><span id=t>L</span></div><div>X</div></div><div style=\"break-before:avoid\"><span id=z>Z</span></div>",
            [(1, 0.0), (1, 40.0)],
        ),
        (
            "<div style=\"height:30px\">S</div><div><div>K</div><div><span id=t>L</span></div></div><div style=\"break-before:avoid\">X</div><div style=\"break-before:avoid\"><span id=z>Z</span></div>",
            [(1, 0.0), (1, 40.0)],
        ),
    ]);
}

#[test]
fn a_first_box_that_holds_no_break_between_boxes_moves_whole() {
    // A paragraph, or a box that avoids breaks inside it, has no break between
    // boxes to take.
    check(&[
        (
            "<div style=\"height:50px\">S</div><div><div><span id=t>L</span><br>X</div></div><div style=\"break-before:avoid\"><span id=z>Z</span></div>",
            [(1, 0.0), (1, 40.0)],
        ),
        (
            "<div style=\"height:50px\">S</div><div style=\"break-inside:avoid\"><div><span id=t>L</span></div><div>X</div></div><div style=\"break-before:avoid\"><span id=z>Z</span></div>",
            [(1, 0.0), (1, 40.0)],
        ),
    ]);
}

#[test]
fn a_box_that_crosses_the_page_end_breaks_inside_instead() {
    // The box the run would start at crosses the page end itself: the break
    // inside it is the later one, and the run stays.
    check(&[
        (
            "<div style=\"height:10px\">S</div><div><div><span id=t>L</span></div><div style=\"height:80px\">X</div></div><div style=\"break-before:avoid\"><span id=z>Z</span></div>",
            [(0, 10.0), (1, 10.0)],
        ),
        (
            "<div style=\"height:30px\">S</div><div><div><span id=t>L</span></div><div style=\"height:60px\">X</div></div><div style=\"break-before:avoid\"><span id=z>Z</span></div>",
            [(0, 30.0), (1, 10.0)],
        ),
    ]);
}

#[test]
fn tables_and_column_flex_containers_break_between_their_rows_and_items() {
    // A table breaks before its last row, repeating its header; a flex
    // container of one column breaks between its items.
    check(&[
        (
            "<div style=\"height:50px\">S</div><div><table style=\"border-spacing:0\"><tr><td style=\"padding:0\"><span id=t>L</span></td></tr><tr><td style=\"padding:0\">X</td></tr></table></div><div style=\"break-before:avoid\"><span id=z>Z</span></div>",
            [(0, 50.0), (1, 20.0)],
        ),
        (
            "<div style=\"height:50px\">S</div><div><table style=\"border-spacing:0\"><thead><tr><td style=\"padding:0\">H</td></tr></thead><tr><td style=\"padding:0\"><span id=t>L</span></td></tr><tr><td style=\"padding:0\">X</td></tr></table></div><div style=\"break-before:avoid\"><span id=z>Z</span></div>",
            [(0, 70.0), (1, 40.0)],
        ),
        (
            "<div style=\"height:50px\">S</div><div style=\"display:flex;flex-direction:column\"><div><span id=t>L</span></div><div>X</div></div><div style=\"break-before:avoid\"><span id=z>Z</span></div>",
            [(0, 50.0), (1, 20.0)],
        ),
    ]);
}

#[test]
fn runs_inside_a_block_start_at_the_last_break_of_their_first_box() {
    // The run lies among the boxes of a block below the body.
    check(&[
        (
            "<div style=\"padding-top:1px\"><div style=\"height:49px\">S</div><div><div><span id=t>L</span></div><div>X</div></div><div style=\"break-before:avoid\"><span id=z>Z</span></div></div>",
            [(0, 50.0), (1, 20.0)],
        ),
        (
            "<div><div style=\"height:50px\">S</div><div><div><span id=t>L</span></div><div>X</div></div><div style=\"break-before:avoid\"><span id=z>Z</span></div></div>",
            [(0, 50.0), (1, 20.0)],
        ),
    ]);
}

#[test]
fn a_first_box_that_clips_at_a_height_of_its_own_moves_whole() {
    // A box that clips its overflow at a height of its own, or contains its size,
    // is monolithic: no break inside it starts the run.
    check(&[
        (
            "<div style=\"height:50px\">S</div><div style=\"overflow:hidden;height:40px\"><div><span id=t>L</span></div><div>X</div></div><div style=\"break-before:avoid\"><span id=z>Z</span></div>",
            [(1, 0.0), (1, 40.0)],
        ),
        (
            "<div style=\"height:50px\">S</div><div style=\"contain:size;height:40px\"><div><span id=t>L</span></div><div>X</div></div><div style=\"break-before:avoid\"><span id=z>Z</span></div>",
            [(1, 0.0), (1, 40.0)],
        ),
    ]);
}

#[test]
fn a_run_too_tall_for_a_page_still_moves_the_part_that_fits() {
    // The boxes tied inside the first box make a run too tall for one page with
    // the box after it. The part up to the lines that the last of them holds
    // for `orphans` still moves when it crosses the page end.
    check(&[
        (
            "<div style=\"height:30px\">S</div><div><div>K</div><div><span id=t>L</span></div><div style=\"break-before:avoid\">X<br>X<br>X<br>X</div></div><div style=\"break-before:avoid\"><span id=z>Z</span></div>",
            [(1, 0.0), (2, 0.0)],
        ),
        (
            "<div style=\"height:30px\">S</div><div><div>K</div><div><span id=t>L</span></div><div style=\"break-before:avoid;orphans:1;widows:1\">X<br>X<br>X<br>X</div></div><div style=\"break-before:avoid\"><span id=z>Z</span></div>",
            [(0, 50.0), (1, 60.0)],
        ),
        (
            "<div style=\"height:30px\">S</div><div><div>K</div><div><div><span id=t>L</span></div><div>X</div></div><div style=\"break-before:avoid\">Y</div></div><div style=\"break-before:avoid\"><span id=z>Z</span></div>",
            [(0, 50.0), (1, 40.0)],
        ),
    ]);
}

#[test]
fn row_groups_and_nested_column_flex_containers_keep_their_breaks() {
    // A break between rows of two row groups counts the constraints on the
    // groups; a row group that avoids breaks inside it only matters when it
    // holds both rows.
    check(&[
        (
            "<div style=\"height:50px\">S</div><div><div style=\"display:flex;flex-direction:column\"><div><span id=t>L</span></div><div>X</div></div></div><div style=\"break-before:avoid\"><span id=z>Z</span></div>",
            [(0, 50.0), (1, 20.0)],
        ),
        (
            "<div style=\"height:50px\">S</div><div><table style=\"border-spacing:0\"><tbody><tr><td style=\"padding:0\"><span id=t>L</span></td></tr></tbody><tbody><tr><td style=\"padding:0\">X</td></tr></tbody></table></div><div style=\"break-before:avoid\"><span id=z>Z</span></div>",
            [(0, 50.0), (1, 20.0)],
        ),
        (
            "<div style=\"height:50px\">S</div><div><table style=\"border-spacing:0\"><tbody><tr><td style=\"padding:0\"><span id=t>L</span></td></tr></tbody><tbody style=\"break-before:avoid\"><tr><td style=\"padding:0\">X</td></tr></tbody></table></div><div style=\"break-before:avoid\"><span id=z>Z</span></div>",
            [(1, 0.0), (1, 40.0)],
        ),
        (
            "<div style=\"height:30px\">S</div><div><table style=\"border-spacing:0\"><tbody style=\"break-inside:avoid\"><tr><td style=\"padding:0\">K</td></tr></tbody><tbody><tr><td style=\"padding:0\"><span id=t>L</span></td></tr><tr><td style=\"padding:0\">X</td></tr></tbody></table></div><div style=\"break-before:avoid\"><span id=z>Z</span></div>",
            [(0, 50.0), (1, 20.0)],
        ),
    ]);
}

#[test]
fn a_table_breaks_before_its_last_body_row() {
    // A table breaks before its last row outside its header and footer
    // groups, unless the row is its only one or the row group holding both
    // rows avoids breaks inside it. A caption does not count.
    check(&[
        (
            "<div style=\"height:50px\">S</div><div><table style=\"border-spacing:0\"><tbody><tr><td style=\"padding:0\"><span id=t>L</span></td></tr><tr><td style=\"padding:0\">X</td></tr></tbody><tfoot><tr><td style=\"padding:0\">F</td></tr></tfoot></table></div><div style=\"break-before:avoid\"><span id=z>Z</span></div>",
            [(0, 50.0), (1, 40.0)],
        ),
        (
            "<div style=\"height:50px\">S</div><div><table style=\"border-spacing:0\"><caption>C</caption><tbody><tr><td style=\"padding:0\"><span id=t>L</span></td></tr><tr><td style=\"padding:0\">X</td></tr></tbody></table></div><div style=\"break-before:avoid\"><span id=z>Z</span></div>",
            [(0, 70.0), (1, 20.0)],
        ),
        (
            "<div style=\"height:70px\">S</div><div><table style=\"border-spacing:0\"><tbody><tr><td style=\"padding:0\"><span id=t>L</span></td></tr></tbody></table></div><div style=\"break-before:avoid\"><span id=z>Z</span></div>",
            [(1, 0.0), (1, 20.0)],
        ),
        (
            "<div style=\"height:50px\">S</div><div><table style=\"border-spacing:0\"><tbody style=\"break-inside:avoid\"><tr><td style=\"padding:0\"><span id=t>L</span></td></tr><tr><td style=\"padding:0\">X</td></tr></tbody></table></div><div style=\"break-before:avoid\"><span id=z>Z</span></div>",
            [(1, 0.0), (1, 40.0)],
        ),
    ]);
}

#[test]
fn a_run_with_a_box_that_holds_a_break_moves_only_when_it_fits() {
    // A box inside the run that holds a break of its own lets the run break
    // there instead when the run does not fit on one page.
    check(&[
        (
            "<div style=\"height:30px\">S</div><div><span id=t>L</span></div><div style=\"break-before:avoid\">X<br>X<br>X<br>X</div><div style=\"break-before:avoid\"><span id=z>Z</span></div>",
            [(0, 30.0), (1, 40.0)],
        ),
        (
            "<div style=\"height:10px\">S</div><div><span id=t>L</span></div><div style=\"break-before:avoid\">X<br>X<br>X</div><div style=\"break-before:avoid\"><span id=z>Z</span></div>",
            [(1, 0.0), (1, 80.0)],
        ),
    ]);
}

#[test]
fn tables_and_deeper_wrappers_in_the_first_box_break_before_their_last_part() {
    // A table alone in the first box, a table tied to the box before it, a
    // table that avoids the break after it, and wrappers nested deeper.
    check(&[
        (
            "<div style=\"height:50px\">S</div><div><table style=\"border-spacing:0;\"><tr><td style=\"padding:0\"><span id=t>L</span></td></tr><tr><td style=\"padding:0\">X</td></tr></table></div><div style=\"break-before:avoid\"><span id=z>Z</span></div>",
            [(0, 50.0), (1, 20.0)],
        ),
        (
            "<div style=\"height:30px\">S</div><div><div style=\"break-after:avoid\">K</div><table style=\"border-spacing:0;\"><tr><td style=\"padding:0\"><span id=t>L</span></td></tr><tr><td style=\"padding:0\">X</td></tr></table></div><div style=\"break-before:avoid\"><span id=z>Z</span></div>",
            [(0, 50.0), (1, 20.0)],
        ),
        (
            "<div style=\"height:30px\">S</div><div><table style=\"border-spacing:0;break-after:avoid\"><tr><td style=\"padding:0\">K</td></tr><tr><td style=\"padding:0\"><span id=t>L</span></td></tr></table><div>X</div></div><div style=\"break-before:avoid\"><span id=z>Z</span></div>",
            [(1, 0.0), (1, 40.0)],
        ),
        (
            "<div style=\"height:30px\">S</div><div><div>K</div><table style=\"border-spacing:0;break-after:avoid\"><tr><td style=\"padding:0\"><span id=t>L</span></td></tr><tr><td style=\"padding:0\">X</td></tr></table><div>Y</div></div><div style=\"break-before:avoid\"><span id=z>Z</span></div>",
            [(0, 50.0), (1, 40.0)],
        ),
        (
            "<div style=\"height:50px\">S</div><div><div><div><div><span id=t>L</span></div><div>X</div></div></div></div><div style=\"break-before:avoid\"><span id=z>Z</span></div>",
            [(0, 50.0), (1, 20.0)],
        ),
        (
            "<div style=\"height:10px\">S</div><div><div>K</div><div><span id=t>L</span></div><div>X<br>X<br>X</div></div><div style=\"break-before:avoid\"><span id=z>Z</span></div>",
            [(0, 30.0), (1, 20.0)],
        ),
    ]);
}

#[test]
fn a_table_of_rows_with_no_row_group_breaks_before_its_last_row() {
    // Rows that are children of the table itself, with no row group.
    check(&[(
        "<div style=\"height:50px\">S</div><div><div style=\"display:table\"><div style=\"display:table-row\"><div style=\"display:table-cell\"><span id=t>L</span></div></div><div style=\"display:table-row\"><div style=\"display:table-cell\">X</div></div></div></div><div style=\"break-before:avoid\"><span id=z>Z</span></div>",
        [(0, 50.0), (1, 20.0)],
    )]);
}

#[test]
fn runs_at_two_levels_that_start_at_the_same_box_hold_the_longer_one() {
    // The runs ending at `Y` and at `Z` both start inside the first box.
    check(&[
        (
            "<div><div>S</div><div><div>K</div><div><div><span id=t>L</span></div><div>X</div></div><div style=\"break-before:avoid\">Y</div></div><div style=\"break-before:avoid\"><span id=z>Z</span></div></div>",
            [(0, 40.0), (1, 40.0)],
        ),
        (
            "<div><div>S</div><div><div>K</div><div><div>X</div><div><span id=t>L</span></div></div><div style=\"break-before:avoid\">Y</div></div><div style=\"break-before:avoid\"><span id=z>Z</span></div></div>",
            [(1, 0.0), (1, 40.0)],
        ),
    ]);
}
