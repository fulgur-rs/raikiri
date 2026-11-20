//! Where the lines around a box with a block size of its own stand in a
//! multicol container whose columns balance, or that fills definite-height
//! columns. A column end slices such a box, as it slices any block (CSS
//! Fragmentation 3, 4.4), so what follows it fills the rest of the column its
//! last piece ends in. The positions are the ones Chromium gives the same
//! documents.

use raikiri_html::{
    DocumentLayout, FontCollectionBuilder, FragmentKind, LayoutOptions, LayoutStatus, PaintEvent,
    RenderResources, layout, parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

/// Ahem's ascent is 0.8em, which puts the top of a 10px glyph 8px above the
/// baseline.
const ASCENT: f32 = 8.0;
/// The advance of one Ahem glyph at 10px.
const ADVANCE: f32 = 10.0;

/// Every word of `body` as `word@x,y`, the left and top of its glyphs, in
/// source order, on an 800x600 page with 10px Ahem lines 10px apart.
fn words(body: &str) -> String {
    let result = lay_out(body);
    let page = result.page(0).expect("one page");
    assert!(result.page(1).is_none(), "the content fits one page");
    let mut words = Vec::new();
    for run in page.text_runs() {
        let mut x = run.origin.0;
        let y = (run.origin.1 - ASCENT).round();
        for word in run.text.split(' ') {
            if !word.trim().is_empty() {
                words.push(format!("{}@{},{}", word.trim(), x.round(), y));
            }
            x += ADVANCE * (word.chars().count() + 1) as f32;
        }
    }
    words.join(" ")
}

/// `body` laid out on an 800x600 page with 10px Ahem lines 10px apart.
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
        "<!doctype html><style>@page{{size:800px 600px;margin:0}}\
         body{{margin:0;font:10px/10px Ahem}}</style><body>{body}</body>"
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

/// A two-column container 200px wide holding `content`, followed by a line.
fn balanced(content: &str) -> String {
    words(&format!(
        "<div style='columns:2;width:200px;column-gap:0'>{content}</div><div>Z</div>"
    ))
}

/// [`balanced`] with columns 45px tall that fill in turn.
fn filled(content: &str) -> String {
    words(&format!(
        "<div style='columns:2;width:200px;column-gap:0;height:45px;column-fill:auto'>\
         {content}</div><div>Z</div>"
    ))
}

const SIZED: &str = "<div style='height:20px'>ab</div>";
const NARROW: &str = "<div style='width:20px'>cd ef gh ij</div>";

#[test]
fn lines_after_a_sized_box_of_lines_fill_its_column() {
    assert_eq!(
        balanced(&format!("{SIZED}{NARROW}")),
        "ab@0,0 cd@0,20 ef@0,30 gh@100,0 ij@100,10 Z@0,40"
    );
    // A box shorter than the lines' pitch leaves room for them below it.
    assert_eq!(
        balanced(&format!("<div style='height:15px'>ab</div>{NARROW}")),
        "ab@0,0 cd@0,15 ef@0,25 gh@100,0 ij@100,10 Z@0,35"
    );
    // A box with two lines of its own.
    assert_eq!(
        balanced(
            "<div style='height:20px'>ab cd</div>\
             <div style='width:20px'>ef gh ij kl mn op</div>"
        ),
        "ab@0,0 cd@30,0 ef@0,20 gh@0,30 ij@100,0 kl@100,10 mn@100,20 op@100,30 Z@0,40"
    );
    // Three columns.
    assert_eq!(
        words(
            "<div style='columns:3;width:300px;column-gap:0'>\
             <div style='height:30px'>ab</div>\
             <div style='width:20px'>cd ef gh ij kl mn</div></div><div>Z</div>"
        ),
        "ab@0,0 cd@100,0 ef@100,10 gh@100,20 ij@200,0 kl@200,10 mn@200,20 Z@0,30"
    );
}

#[test]
fn orphans_and_widows_still_hold_after_a_sized_box() {
    assert_eq!(
        balanced(
            "<div style='height:20px'>ab</div>\
             <div style='width:20px;orphans:1;widows:1'>cd ef gh ij</div>"
        ),
        "ab@0,0 cd@0,20 ef@100,0 gh@100,10 ij@100,20 Z@0,30"
    );
    // Margins between the box and the lines.
    assert_eq!(
        balanced(
            "<div style='height:20px;margin-bottom:5px'>ab</div>\
             <div style='width:20px;margin-top:10px'>cd ef gh ij</div>"
        ),
        "ab@0,0 cd@100,0 ef@100,10 gh@100,20 ij@100,30 Z@0,40"
    );
}

#[test]
fn a_column_end_slices_a_sized_box_of_lines() {
    // The box runs 5px into the second column, and the lines start below it.
    assert_eq!(
        balanced(&format!("<div style='height:50px'>ab</div>{NARROW}")),
        "ab@0,0 cd@100,5 ef@100,15 gh@100,25 ij@100,35 Z@0,45"
    );
    assert_eq!(
        balanced(
            "<div style='width:20px'>cd ef</div><div style='height:20px'>ab</div>\
             <div style='width:20px'>gh ij kl</div>"
        ),
        "cd@0,0 ef@0,10 ab@0,20 gh@100,5 ij@100,15 kl@100,25 Z@0,35"
    );
    assert_eq!(
        balanced(
            "<div style='height:60px;width:20px'>ab cd</div>\
             <div style='width:20px'>kl mn</div>"
        ),
        "ab@0,0 cd@0,10 kl@100,20 mn@100,30 Z@0,40"
    );
    assert_eq!(
        balanced(
            "<div style='width:20px'>kl mn</div>\
             <div style='height:60px;width:20px'>ab cd</div>"
        ),
        "kl@0,0 mn@0,10 ab@0,20 cd@0,30 Z@0,40"
    );
}

#[test]
fn a_sized_box_after_the_lines_starts_the_next_column() {
    assert_eq!(
        balanced(&format!("{NARROW}{SIZED}")),
        "cd@0,0 ef@0,10 gh@0,20 ij@0,30 ab@100,0 Z@0,40"
    );
    assert_eq!(
        balanced("<div style='width:20px'>cd ef gh</div><div style='height:30px'>ab</div>"),
        "cd@0,0 ef@0,10 gh@0,20 ab@100,0 Z@0,30"
    );
}

#[test]
fn a_sized_box_with_no_lines_leaves_its_column_to_the_lines() {
    assert_eq!(
        balanced(&format!("<div style='height:20px'></div>{NARROW}")),
        "cd@0,20 ef@0,30 gh@100,0 ij@100,10 Z@0,40"
    );
}

#[test]
fn definite_columns_slice_a_sized_box_of_lines() {
    assert_eq!(
        filled(&format!("<div style='height:50px'>ab</div>{NARROW}")),
        "ab@0,0 cd@100,5 ef@100,15 gh@100,25 ij@100,35 Z@0,45"
    );
    assert_eq!(
        filled(
            "<div style='width:20px'>cd ef</div><div style='height:40px'>ab</div>\
             <div style='width:20px'>gh ij</div>"
        ),
        "cd@0,0 ef@0,10 ab@0,20 gh@100,15 ij@100,25 Z@0,45"
    );
    assert_eq!(
        filled(&format!("<div style='height:30px'>ab</div>{NARROW}")),
        "ab@0,0 cd@100,0 ef@100,10 gh@100,20 ij@100,30 Z@0,45"
    );
}

#[test]
fn a_sized_box_whose_lines_a_column_end_would_cut_goes_whole_to_the_next_column() {
    assert_eq!(
        filled(
            "<div style='width:20px'>cd ef gh</div>\
             <div style='height:40px;width:20px'>ab cd</div>\
             <div style='width:20px'>ij kl</div>"
        ),
        "cd@0,0 ef@0,10 gh@0,20 ab@100,0 cd@100,10 ij@200,0 kl@200,10 Z@0,45"
    );
    assert_eq!(
        filled(
            "<div style='width:20px'>cd ef gh ij</div>\
             <div style='height:20px;width:20px'>ab cd</div>\
             <div style='width:20px'>kl</div>"
        ),
        "cd@0,0 ef@0,10 gh@0,20 ij@0,30 ab@100,0 cd@100,10 kl@100,20 Z@0,45"
    );
}

#[test]
fn a_sized_box_of_lines_in_a_wrapper_joins_the_flow() {
    assert_eq!(
        balanced(&format!("<div>{SIZED}{NARROW}</div>")),
        "ab@0,0 cd@0,20 ef@0,30 gh@100,0 ij@100,10 Z@0,40"
    );
    assert_eq!(
        balanced(&format!("<div>{NARROW}{SIZED}</div>")),
        "cd@0,0 ef@0,10 gh@0,20 ij@0,30 ab@100,0 Z@0,40"
    );
    assert_eq!(
        balanced(&format!("<div>{SIZED}</div>{NARROW}")),
        "ab@0,0 cd@0,20 ef@0,30 gh@100,0 ij@100,10 Z@0,40"
    );
    assert_eq!(
        balanced(&format!(
            "<div style='padding-top:5px'>{SIZED}{NARROW}</div>"
        )),
        "ab@0,5 cd@100,0 ef@100,10 gh@100,20 ij@100,30 Z@0,40"
    );
    assert_eq!(
        balanced(&format!(
            "<div><div style='height:15px'>ab</div>{NARROW}</div>"
        )),
        "ab@0,0 cd@0,15 ef@0,25 gh@100,0 ij@100,10 Z@0,35"
    );
}

#[test]
fn the_pieces_of_a_sliced_box_of_lines_fill_their_columns() {
    let result = lay_out(
        "<div style='columns:2;width:200px;column-gap:0'>\
         <div style='height:50px;background:green'>ab</div>\
         <div style='width:20px'>cd ef gh ij</div></div>",
    );
    let page = result.page(0).expect("one page");
    // The column boxes are 100px wide; the sized box is the only one.
    let pieces: Vec<_> = page
        .fragments()
        .filter(|fragment| fragment.kind() == FragmentKind::Box && fragment.rect().width == 100.0)
        .map(|fragment| {
            let rect = fragment.rect();
            (rect.x, rect.y, rect.height)
        })
        .collect();
    assert_eq!(pieces, [(0.0, 0.0, 45.0), (100.0, 0.0, 5.0)]);
}

#[test]
fn a_column_holding_only_a_piece_of_a_sliced_box_has_a_rule_before_it() {
    let content = "<div style='width:20px'>cd ef</div><div style='height:60px'>ab</div>";
    assert_eq!(balanced(content), "cd@0,0 ef@0,10 ab@0,20 Z@0,40");
    let result = lay_out(&format!(
        "<div style='columns:2;width:200px;column-gap:0;column-rule:2px solid'>{content}</div>"
    ));
    let rules = result
        .page(0)
        .expect("one page")
        .paint_order()
        .iter()
        .filter(|event| matches!(event, PaintEvent::ColumnRule(_)))
        .count();
    assert_eq!(rules, 1);
}

#[test]
fn a_sized_box_of_lines_in_a_wrapper_keeps_its_height() {
    // The wrapper's piece in the first column, then the sized box.
    for container in [
        "columns:2;width:200px;column-gap:0",
        "columns:2;width:200px;column-gap:0;height:45px;column-fill:auto",
    ] {
        let result = lay_out(&format!(
            "<div style='{container}'><div>\
             <div style='height:20px;background:green'>ab</div>{NARROW}</div></div>"
        ));
        let page = result.page(0).expect("one page");
        let first_column: Vec<_> = page
            .fragments()
            .filter(|fragment| fragment.kind() == FragmentKind::Box)
            .map(|fragment| {
                let rect = fragment.rect();
                (rect.x, rect.y, rect.width, rect.height)
            })
            .filter(|&(x, y, width, _)| x == 0.0 && y == 0.0 && width == 100.0)
            .collect();
        assert_eq!(
            first_column[1],
            (0.0, 0.0, 100.0, 20.0),
            "{container}: {first_column:?}"
        );
    }
}
