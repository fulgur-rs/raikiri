//! A table cell's own `width` says how wide the cell wants to be, but the
//! column can never be narrower than what the cell's content cannot break
//! (CSS 2.1 17.5.2.2): a word, an inline-block, an image, or a `nowrap` line.
//! The authored width therefore sets the preferred width above that floor, and
//! a column whose only authored width is narrower than the content is as wide
//! as the content.
//!
//! The expected boxes are the ones Chromium computes for the same markup in an
//! 800px by 600px viewport with the body margin of 8px, in Ahem at 10px
//! (a glyph is 10px wide, so the 18-letter word is 180px). Every cell has 5px
//! of padding and a 1px border, and the table `border-spacing` is 2px. Each
//! box is `(id, x, y, width, height)`.

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

const AHEM: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/data/text-autospace/Ahem.ttf"
));

/// The border boxes of the elements of `body` that have an `id`, on the first
/// page, as `(id, x, y, width, height)`.
fn boxes(body: &str) -> Vec<(String, f32, f32, f32, f32)> {
    let html = format!(
        "<!DOCTYPE html><style>@page {{ size: 800px 600px; margin: 0 }} \
         body {{ margin: 8px; font: 10px/10px Ahem }} \
         table {{ border-collapse: separate; border-spacing: 2px }} \
         td {{ padding: 5px; border: 1px solid }}</style>{body}"
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
        .filter_map(|fragment| {
            let id = dom.attr(fragment.node(), "id")?;
            let rect = fragment.rect();
            Some((id.to_owned(), rect.x, rect.y, rect.width, rect.height))
        })
        .collect()
}

/// Layout units are 1/64px; allow a little more for the 32-bit sums.
const TOLERANCE: f32 = 0.04;

/// The `(id, x, y, width, height)` a case expects.
type Expected<'a> = &'a [(&'a str, f32, f32, f32, f32)];

/// A label, the markup of a case and the boxes it expects.
type Case<'a> = (&'a str, &'a str, Expected<'a>);

/// Lays out each case and compares the boxes it names with the expected ones.
fn check_boxes(cases: &[Case<'_>]) {
    for &(label, body, expected) in cases {
        let actual = boxes(body);
        for &(id, x, y, width, height) in expected {
            let (_, ax, ay, aw, ah) = actual
                .iter()
                .find(|(name, ..)| name == id)
                .unwrap_or_else(|| panic!("{label}: no fragment for #{id}"));
            let matches = |actual: f32, expected: f32| (actual - expected).abs() <= TOLERANCE;
            assert!(
                matches(*ax, x) && matches(*ay, y) && matches(*aw, width) && matches(*ah, height),
                "{label}: #{id} is ({ax}, {ay}, {aw}, {ah}), expected ({x}, {y}, {width}, {height})"
            );
        }
    }
}

/// A cell whose `width` is narrower than its word is as wide as the word, and
/// a `width` wider than the word still sets the column.
#[test]
fn an_authored_width_does_not_squeeze_a_cell_below_its_unbreakable_content() {
    check_boxes(&[
        (
            "width 20px, word 180px",
            "<table id=t><tr><td id=a>Row 1 Left</td><td id=c style='width:20px'>Supersuperlongword</td></tr></table>",
            &[
                ("t", 8.0, 8.0, 310.0, 26.0),
                ("a", 10.0, 10.0, 112.0, 22.0),
                ("c", 124.0, 10.0, 192.0, 22.0),
            ],
        ),
        (
            "width 100px, word 180px",
            "<table id=t><tr><td id=a>Row 1 Left</td><td id=c style='width:100px'>Supersuperlongword</td></tr></table>",
            &[
                ("t", 8.0, 8.0, 310.0, 26.0),
                ("a", 10.0, 10.0, 112.0, 22.0),
                ("c", 124.0, 10.0, 192.0, 22.0),
            ],
        ),
        (
            "width 200px, word 180px",
            "<table id=t><tr><td id=a>Row 1 Left</td><td id=c style='width:200px'>Supersuperlongword</td></tr></table>",
            &[
                ("t", 8.0, 8.0, 330.0, 26.0),
                ("a", 10.0, 10.0, 112.0, 22.0),
                ("c", 124.0, 10.0, 212.0, 22.0),
            ],
        ),
    ]);
}

/// With `box-sizing: border-box` the authored width includes the padding and
/// border, and the floor is still the content plus them.
#[test]
fn a_border_box_width_gives_way_to_the_content_too() {
    check_boxes(&[(
        "border-box 100px, word 180px",
        "<style>td { box-sizing: border-box }</style>\
         <table id=t><tr><td id=a>Row 1 Left</td><td id=c style='width:100px'>Supersuperlongword</td></tr></table>",
        &[
            ("t", 8.0, 8.0, 310.0, 26.0),
            ("a", 10.0, 10.0, 112.0, 22.0),
            ("c", 124.0, 10.0, 192.0, 22.0),
        ],
    )]);
}

/// The row is as tall as the wider column makes it: the paragraphs in the cell
/// lay out at the width of the column, not of the authored `width`.
#[test]
fn the_cell_lays_out_at_the_widened_column_width() {
    check_boxes(&[(
        "paragraphs in a width 100px cell",
        "<table id=t><tr><td id=a>Row 1 Left</td><td id=c style='width:100px'>\
         <p id=p1>Supersuperlongword</p><p id=p2>row with lots and lots of text</p></td></tr></table>",
        &[
            ("t", 8.0, 8.0, 310.0, 76.0),
            ("a", 10.0, 10.0, 112.0, 72.0),
            ("c", 124.0, 10.0, 192.0, 72.0),
            ("p1", 130.0, 26.0, 180.0, 10.0),
            ("p2", 130.0, 46.0, 180.0, 20.0),
        ],
    )]);
}

/// Breakable text keeps the authored width as its preferred width, with the
/// longest word as the floor.
#[test]
fn breakable_text_keeps_its_authored_width_above_the_longest_word() {
    check_boxes(&[
        (
            "width 30px, words of 50px at most",
            "<table id=t><tr><td id=a>Row 1 Left</td><td id=c style='width:30px'>row with lots and lots of text</td></tr></table>",
            &[
                ("t", 8.0, 8.0, 170.0, 86.0),
                ("a", 10.0, 10.0, 112.0, 82.0),
                ("c", 124.0, 10.0, 52.0, 82.0),
            ],
        ),
        (
            "width 40px beside an unauthored cell",
            "<table id=t><tr><td id=a style='width:40px'>aa bb cc dd</td><td id=c>Row 1 Left</td></tr></table>",
            &[
                ("t", 8.0, 8.0, 170.0, 56.0),
                ("a", 10.0, 10.0, 52.0, 52.0),
                ("c", 64.0, 10.0, 112.0, 52.0),
            ],
        ),
    ]);
}

/// Every cell of a row with an authored width gets the same floor, and the
/// columns of the other rows follow.
#[test]
fn each_authored_width_in_a_row_keeps_its_floor() {
    check_boxes(&[
        (
            "two cells with width 100px",
            "<table id=t><tr><td id=a style='width:100px'>Supersuperlongword</td><td id=c style='width:100px'>Superlong</td></tr></table>",
            &[
                ("t", 8.0, 8.0, 310.0, 26.0),
                ("a", 10.0, 10.0, 192.0, 22.0),
                ("c", 204.0, 10.0, 112.0, 22.0),
            ],
        ),
        (
            "a second row under the narrowed cell",
            "<table id=t><tr><td id=a>x</td><td id=c style='width:100px'>Supersuperlongword</td></tr>\
             <tr><td id=b>y</td><td id=d>z</td></tr></table>",
            &[
                ("t", 8.0, 8.0, 220.0, 50.0),
                ("a", 10.0, 10.0, 22.0, 22.0),
                ("c", 34.0, 10.0, 192.0, 22.0),
                ("b", 10.0, 34.0, 22.0, 22.0),
                ("d", 34.0, 34.0, 192.0, 22.0),
            ],
        ),
        (
            "a cell in the second row sets the floor of the column",
            "<table id=t><tr><td id=a>x</td><td id=c style='width:100px'>ab</td></tr>\
             <tr><td id=b>y</td><td id=d style='width:60px'>Supersuperlongword</td></tr></table>",
            &[
                ("t", 8.0, 8.0, 220.0, 50.0),
                ("a", 10.0, 10.0, 22.0, 22.0),
                ("c", 34.0, 10.0, 192.0, 22.0),
                ("b", 10.0, 34.0, 22.0, 22.0),
                ("d", 34.0, 34.0, 192.0, 22.0),
            ],
        ),
    ]);
}

/// A cell that spans columns shares its floor among them in the same way.
#[test]
fn a_spanning_cell_with_an_authored_width_keeps_its_floor() {
    check_boxes(&[(
        "colspan 2, width 50px, word 180px",
        "<table id=t><tr><td id=a colspan=2 style='width:50px'>Supersuperlongword</td></tr>\
         <tr><td id=b>b</td><td id=c>c</td></tr></table>",
        &[
            ("t", 8.0, 8.0, 196.0, 50.0),
            ("a", 10.0, 10.0, 192.0, 22.0),
            ("b", 10.0, 34.0, 95.0, 22.0),
            ("c", 107.0, 34.0, 95.0, 22.0),
        ],
    )]);
}

/// A `nowrap` line and an inline-block cannot break either, so they are the
/// floor of the column.
#[test]
fn nowrap_text_and_atomic_inlines_keep_their_width_too() {
    check_boxes(&[
        (
            "nowrap line of 170px under width 100px",
            "<table id=t><tr><td id=a>Row 1 Left</td><td id=c style='width:100px;white-space:nowrap'>aa bb cc dd ee ff</td></tr></table>",
            &[
                ("t", 8.0, 8.0, 300.0, 26.0),
                ("a", 10.0, 10.0, 112.0, 22.0),
                ("c", 124.0, 10.0, 182.0, 22.0),
            ],
        ),
        (
            "inline-block of 90px under width 20px",
            "<table id=t><tr><td id=a>Row 1 Left</td><td id=c style='width:20px'>\
             <div id=i style='width:90px;height:10px;display:inline-block'></div></td></tr></table>",
            &[
                ("t", 8.0, 8.0, 220.0, 28.0),
                ("a", 10.0, 10.0, 112.0, 24.0),
                ("c", 124.0, 10.0, 102.0, 24.0),
                ("i", 130.0, 16.0, 90.0, 10.0),
            ],
        ),
    ]);
}

/// A table with a `width` that leaves less room than the cells want is still
/// shared out the way it was: the floor only lifts the minimum, and the
/// remaining room goes to the preferred widths.
#[test]
fn a_table_width_still_shares_the_room_by_preferred_width() {
    check_boxes(&[
        (
            "table 300px, width 200px against a word of 180px",
            "<table id=t style='width:300px'><tr><td id=a>Row 1 Left</td><td id=c style='width:200px'>Supersuperlongword</td></tr></table>",
            &[
                ("t", 8.0, 8.0, 300.0, 36.0),
                ("a", 10.0, 10.0, 82.0, 32.0),
                ("c", 94.0, 10.0, 212.0, 32.0),
            ],
        ),
        (
            "table 400px, two cells with width 150px",
            "<table id=t style='width:400px'><tr><td id=a style='width:150px'>Supersuperlongword</td><td id=c style='width:150px'>Supersuperlongword</td></tr></table>",
            &[
                ("t", 8.0, 8.0, 400.0, 26.0),
                ("a", 10.0, 10.0, 197.0, 22.0),
                ("c", 209.0, 10.0, 197.0, 22.0),
            ],
        ),
        (
            "table 400px, 30% width against a word of 180px",
            "<table id=t style='width:400px'><tr><td id=a>Row 1 Left</td><td id=c style='width:30%'>Supersuperlongword</td></tr></table>",
            &[
                ("t", 8.0, 8.0, 400.0, 26.0),
                ("a", 10.0, 10.0, 202.0, 22.0),
                ("c", 214.0, 10.0, 192.0, 22.0),
            ],
        ),
    ]);
}
