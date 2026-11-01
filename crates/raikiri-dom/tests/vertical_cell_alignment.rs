//! A cell in a vertical writing mode aligns its content by `vertical-align`
//! along the block axis of the cell, which is its physical width: the free room
//! goes after the content at the block-start side for `top` and `baseline`,
//! splits for `middle`, and comes before the content for `bottom`. A cell in a
//! horizontal table does the same as one in a vertical table.
//!
//! The expected boxes are the ones Chromium computes for the same markup in an
//! 800px by 600px viewport, with the body margin of 8px and a font of 10px
//! Ahem: each is an element's left, top, width and height. The table is
//! positioned so that it is the containing block of the absolute box.

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

const AHEM: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/data/text-autospace/Ahem.ttf"
));

/// The left, top, width and height of the element `id` in `body`, on the first
/// page of 800px by 600px.
fn border_box(body: &str, id: &str) -> [f32; 4] {
    let html = format!(
        "<!DOCTYPE html><style>@page {{ size: 800px 600px; margin: 0 }} \
         body {{ margin: 8px; font: 10px/10px Ahem }} div {{ box-sizing: border-box }}</style>{body}"
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
    let page = result.pages().next().expect("a page");
    let dom = page.dom();
    page.fragments()
        .find(|fragment| dom.attr(fragment.node(), "id") == Some(id))
        .map(|fragment| {
            let rect = fragment.rect();
            [rect.x, rect.y, rect.width, rect.height]
        })
        .unwrap_or_else(|| panic!("no fragment for #{id} in {body}"))
}

/// Layout units are 1/64px; allow a little more for the 32-bit sums.
const TOLERANCE: f32 = 0.04;

/// Where the table with the writing mode is.
#[derive(Clone, Copy, Debug)]
enum Table {
    /// A horizontal table whose cell is in the vertical writing mode.
    Horizontal,
    /// A table in the vertical writing mode, which its cell inherits.
    Vertical,
}

/// A table `t` of one row `r` and one cell `c` of 80px by 80px, aligned by
/// `align`, holding `content`, in the writing mode `mode`.
fn cell(table: Table, mode: &str, align: &str, content: &str) -> String {
    let (table_mode, cell_mode) = match table {
        Table::Horizontal => (String::new(), format!("writing-mode: {mode};")),
        Table::Vertical => (format!("writing-mode: {mode};"), String::new()),
    };
    format!(
        "<div id=t style='display: table; position: relative; {table_mode}'>\
         <div id=r style='display: table-row'>\
         <div id=c style='display: table-cell; width: 80px; height: 80px; \
         vertical-align: {align}; {cell_mode}'>{content}</div></div></div>"
    )
}

/// Compares the boxes of the elements `expected` names, reporting every
/// mismatch at once.
fn check(label: &str, markup: &str, expected: &[(&str, [f32; 4])]) -> Vec<String> {
    let mut mismatches = Vec::new();
    for (id, want) in expected {
        let found = border_box(markup, id);
        if !found
            .iter()
            .zip(want)
            .all(|(got, want)| (got - want).abs() <= TOLERANCE)
        {
            mismatches.push(format!(
                "{label}: #{id} is {found:?}, expected {want:?} (left, top, width, height)"
            ));
        }
    }
    mismatches
}

/// The left of the content of an 80px cell for each alignment, in
/// `vertical-lr` and `vertical-rl`.
type Lefts = [(&'static str, f32, f32); 4];

/// A line of text in a block `p`: 10px wide and as high as the cell.
const TEXT: Lefts = [
    ("baseline", 8.0, 78.0),
    ("top", 8.0, 78.0),
    ("middle", 43.0, 43.0),
    ("bottom", 78.0, 8.0),
];

/// A box `p` of 30px by 20px.
const BOX: Lefts = [
    ("baseline", 8.0, 58.0),
    ("top", 8.0, 58.0),
    ("middle", 33.0, 33.0),
    ("bottom", 58.0, 8.0),
];

fn check_lefts(table: Table, content: &str, size: [f32; 2], lefts: Lefts) {
    let mut mismatches = Vec::new();
    for (align, lr, rl) in lefts {
        for (mode, left) in [("vertical-lr", lr), ("vertical-rl", rl)] {
            let markup = cell(table, mode, align, content);
            mismatches.extend(check(
                &format!("{table:?} table, {mode}, {align}"),
                &markup,
                &[("p", [left, 8.0, size[0], size[1]])],
            ));
        }
    }
    assert!(mismatches.is_empty(), "{}", mismatches.join("\n"));
}

#[test]
fn a_vertical_cell_in_a_horizontal_table_aligns_on_its_block_axis() {
    check_lefts(Table::Horizontal, "<div id=p>x</div>", [10.0, 80.0], TEXT);
    check_lefts(
        Table::Horizontal,
        "<div id=p style='width: 30px; height: 20px'></div>",
        [30.0, 20.0],
        BOX,
    );
}

#[test]
fn a_vertical_cell_in_a_vertical_table_aligns_the_same_way() {
    check_lefts(Table::Vertical, "<div id=p>x</div>", [10.0, 80.0], TEXT);
    check_lefts(
        Table::Vertical,
        "<div id=p style='width: 30px; height: 20px'></div>",
        [30.0, 20.0],
        BOX,
    );
}

#[test]
fn text_blocks_in_a_vertical_rl_cell_move_off_the_right_side() {
    // A block with text sits against the right side of a `vertical-rl` cell,
    // and the room left by `middle` and `bottom` goes to its left.
    let wide = "<div id=p style='width: 30px'>x</div>";
    let two = "<div id=p>x</div><div id=q>y</div>";
    // (alignment, left of `p` and `q` in vertical-rl, and in vertical-lr)
    let wide_lefts = [
        ("top", 58.0, 8.0),
        ("middle", 33.0, 33.0),
        ("bottom", 8.0, 58.0),
    ];
    let two_lefts = [
        ("top", (78.0, 68.0), (8.0, 18.0)),
        ("middle", (48.0, 38.0), (38.0, 48.0)),
        ("bottom", (18.0, 8.0), (68.0, 78.0)),
    ];
    let mut mismatches = Vec::new();
    for table in [Table::Horizontal, Table::Vertical] {
        for (align, rl, lr) in wide_lefts {
            for (mode, left) in [("vertical-rl", rl), ("vertical-lr", lr)] {
                mismatches.extend(check(
                    &format!("{table:?} table, a block of 30px, {mode}, {align}"),
                    &cell(table, mode, align, wide),
                    &[("p", [left, 8.0, 30.0, 80.0])],
                ));
            }
        }
        for (align, rl, lr) in two_lefts {
            for (mode, (p, q)) in [("vertical-rl", rl), ("vertical-lr", lr)] {
                mismatches.extend(check(
                    &format!("{table:?} table, two blocks, {mode}, {align}"),
                    &cell(table, mode, align, two),
                    &[("p", [p, 8.0, 10.0, 80.0]), ("q", [q, 8.0, 10.0, 80.0])],
                ));
            }
        }
    }
    assert!(mismatches.is_empty(), "{}", mismatches.join("\n"));
}

#[test]
fn a_box_and_a_text_block_in_a_vertical_rl_cell_move_together() {
    let mixed = "<div id=p>x</div><div id=q style='width: 20px; height: 20px'></div>";
    let nested = "<div id=p><div id=q>x</div></div>";
    let boxed = "<div id=p style='width: 30px; height: 20px'>\
                 <div id=q style='width: 10px; height: 10px'></div></div>";
    let mut mismatches = Vec::new();
    for table in [Table::Horizontal, Table::Vertical] {
        mismatches.extend(check(
            &format!("{table:?} table, a text block and a box"),
            &cell(table, "vertical-rl", "bottom", mixed),
            &[
                ("p", [28.0, 8.0, 10.0, 80.0]),
                ("q", [8.0, 8.0, 20.0, 20.0]),
            ],
        ));
        mismatches.extend(check(
            &format!("{table:?} table, a block in a block"),
            &cell(table, "vertical-rl", "bottom", nested),
            &[("p", [8.0, 8.0, 10.0, 80.0]), ("q", [8.0, 8.0, 10.0, 80.0])],
        ));
        mismatches.extend(check(
            &format!("{table:?} table, a box in a box"),
            &cell(table, "vertical-rl", "bottom", boxed),
            &[
                ("p", [8.0, 8.0, 30.0, 20.0]),
                ("q", [28.0, 8.0, 10.0, 10.0]),
            ],
        ));
    }
    assert!(mismatches.is_empty(), "{}", mismatches.join("\n"));
}

#[test]
fn an_out_of_flow_box_in_a_vertical_cell_stays_at_its_insets() {
    // The cell is the containing block of `q`: aligning the in-flow content
    // moves `p` and leaves `q` where its insets put it.
    let content = "<div id=p style='width: 30px; height: 20px'></div>\
                   <div id=q style='position: absolute; left: 3px; top: 4px; \
                   width: 5px; height: 5px'></div>";
    // (alignment, left of `p` in vertical-rl, and in vertical-lr)
    let lefts = [
        ("top", 58.0, 8.0),
        ("middle", 33.0, 33.0),
        ("bottom", 8.0, 58.0),
    ];
    let mut mismatches = Vec::new();
    for table in [Table::Horizontal, Table::Vertical] {
        for (align, rl, lr) in lefts {
            for (mode, left) in [("vertical-rl", rl), ("vertical-lr", lr)] {
                mismatches.extend(check(
                    &format!("{table:?} table, an absolute box, {mode}, {align}"),
                    &cell(table, mode, align, content),
                    &[
                        ("p", [left, 8.0, 30.0, 20.0]),
                        ("q", [11.0, 12.0, 5.0, 5.0]),
                    ],
                ));
            }
        }
    }
    assert!(mismatches.is_empty(), "{}", mismatches.join("\n"));
}
