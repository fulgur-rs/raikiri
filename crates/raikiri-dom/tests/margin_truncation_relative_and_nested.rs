//! A page edge that falls in the margin between two sibling blocks cuts that
//! margin, and the block after it starts the next page (CSS Fragmentation 3
//! §5.2). This also holds when either block is relatively positioned or
//! sticky, as the page edge is found where the flow puts the blocks, not where
//! their offsets paint them, and when the two blocks sit inside another block,
//! relatively positioned or not.
//!
//! In each case a block of 80px is followed by a 20px block `b` with a 30px
//! margin above it, which the edge of the first page at 100px cuts, and by a
//! 10px block `c`. The pages are 100px by 100px with no margin, and the
//! expected boxes are the ones Chromium 153 prints for the same markup.

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

const AHEM: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/data/text-autospace/Ahem.ttf"
));

/// A box of an element with an `id`: its page, from 0, and its left, top,
/// width and height on that page.
type Placed = (usize, f32, f32, f32, f32);

/// The number of pages and the boxes of each element with an `id`.
fn pages_of(body: &str) -> (usize, Vec<(String, Placed)>) {
    let html = format!(
        "<!DOCTYPE html><style>@page {{ size: 100px 100px; margin: 0 }} \
         body {{ margin: 0; font: 20px/30px Ahem }} div {{ box-sizing: border-box }}</style>{body}"
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
    let mut boxes = Vec::new();
    let mut pages = 0;
    for (index, page) in result.pages().enumerate() {
        pages = index + 1;
        let dom = page.dom();
        for fragment in page.fragments() {
            if let Some(id) = dom.attr(fragment.node(), "id") {
                let rect = fragment.rect();
                boxes.push((
                    id.to_owned(),
                    (index, rect.x, rect.y, rect.width, rect.height),
                ));
            }
        }
    }
    (pages, boxes)
}

/// Checks the block-axis extents of `#b` and `#c` on each page:
/// `(id, page, top, bottom)`.
fn check(label: &str, body: &str, expected: &[(&str, usize, f32, f32)]) {
    let (pages, boxes) = pages_of(body);
    let got: Vec<(&str, usize, f32, f32)> = boxes
        .iter()
        .filter(|(id, _)| id == "b" || id == "c")
        .map(|(id, (page, _, y, _, height))| (id.as_str(), *page, *y, y + height))
        .collect();
    assert_eq!(pages, 2, "{label}: page count, got {got:?}");
    let near = |a: f32, b: f32| (a - b).abs() <= 0.04;
    assert!(
        got.len() == expected.len()
            && got
                .iter()
                .zip(expected)
                .all(|(g, e)| g.0 == e.0 && g.1 == e.1 && near(g.2, e.2) && near(g.3, e.3)),
        "{label}: got {got:?}, expected {expected:?}"
    );
}

/// The 80px block before `b`, with `style` added.
fn first(style: &str) -> String {
    format!("<div style=\"height:80px;{style}\"></div>")
}

/// The block `b` with the 30px margin above it, with `style` added.
fn b(style: &str) -> String {
    format!("<div id=b style=\"margin-top:30px;height:20px;{style}\"></div>")
}

const C: &str = "<div id=c style=\"height:10px\"></div>";

/// A relative offset paints `b` away from the top of the second page, where
/// the cut margin puts it; `c` follows `b` where the flow puts it.
#[test]
fn a_relatively_positioned_block_loses_the_margin_the_page_edge_cuts() {
    check(
        "top:25px",
        &format!("{}{}{C}", first(""), b("position:relative;top:25px")),
        &[("b", 1, 25.0, 45.0), ("c", 1, 20.0, 30.0)],
    );
    // An offset up into the first page paints `b` there, whole.
    check(
        "top:-50px",
        &format!("{}{}{C}", first(""), b("position:relative;top:-50px")),
        &[("b", 0, 50.0, 70.0), ("c", 1, 20.0, 30.0)],
    );
    // An offset across the page edge paints `b` on both pages.
    check(
        "top:-15px",
        &format!("{}{}{C}", first(""), b("position:relative;top:-15px")),
        &[
            ("b", 0, 85.0, 100.0),
            ("b", 1, 0.0, 5.0),
            ("c", 1, 20.0, 30.0),
        ],
    );
    // A sticky block is in the flow like a relative one.
    check(
        "sticky",
        &format!("{}{}{C}", first(""), b("position:sticky;top:0")),
        &[("b", 1, 0.0, 20.0), ("c", 1, 20.0, 30.0)],
    );
}

/// The margin is measured from the bottom of the block before `b` in the
/// flow, also when an offset paints that block lower.
#[test]
fn the_margin_is_measured_from_where_the_flow_puts_the_block_before() {
    check(
        "static after relative",
        &format!("{}{}{C}", first("position:relative;top:15px"), b("")),
        &[("b", 1, 0.0, 20.0), ("c", 1, 20.0, 30.0)],
    );
    check(
        "relative after relative",
        &format!(
            "{}{}{C}",
            first("position:relative;top:15px"),
            b("position:relative;top:5px")
        ),
        &[("b", 1, 5.0, 25.0), ("c", 1, 20.0, 30.0)],
    );
}

/// The page edge cuts the margin between two blocks inside another block
/// too, whatever `b` holds, and a relative offset on that block paints both
/// lower.
#[test]
fn the_margin_between_blocks_inside_a_block_is_cut() {
    let wrapped = |wrapper: &str, inner: &str, c_inside: bool| {
        let (inside, after) = if c_inside { (C, "") } else { ("", C) };
        format!(
            "<div style=\"{wrapper}\">{}{inner}{inside}</div>{after}",
            first("")
        )
    };
    let relative = "position:relative;top:20px";
    for (label, inner) in [
        ("empty", b("")),
        (
            "text",
            "<div id=b style=\"margin-top:30px;font:20px/20px Ahem\">x</div>".to_owned(),
        ),
        ("avoid", b("break-inside:avoid")),
        (
            "image",
            "<img id=b style=\"display:block;margin-top:30px;height:20px;width:10px\">".to_owned(),
        ),
    ] {
        check(
            &format!("{label} in a static block"),
            &wrapped("", &inner, false),
            &[("b", 1, 0.0, 20.0), ("c", 1, 20.0, 30.0)],
        );
        check(
            &format!("{label} in a relative block"),
            &wrapped(relative, &inner, false),
            &[("b", 1, 20.0, 40.0), ("c", 1, 20.0, 30.0)],
        );
    }
    // The margin may be the block-end margin of the block before `b`.
    check(
        "margin below the block before",
        "<div><div style=\"height:80px;margin-bottom:30px\"></div>\
         <div id=b style=\"height:20px\"></div></div><div id=c style=\"height:10px\"></div>",
        &[("b", 1, 0.0, 20.0), ("c", 1, 20.0, 30.0)],
    );
    check(
        "c in the relative block",
        &wrapped(relative, &b(""), true),
        &[("b", 1, 20.0, 40.0), ("c", 1, 40.0, 50.0)],
    );
}

/// In an item of a grid with two columns, the margin stays as it is, and the
/// item beside it and the block after the grid keep their places (Chromium
/// prints `y` and `c` there too): cutting it would move the shared flow.
#[test]
fn a_margin_in_a_grid_item_leaves_the_items_beside_it_in_place() {
    let body = format!(
        "<div style=\"display:grid;grid-template-columns:50px 50px\"><div>{}{}</div>\
         <div><div style=\"height:150px\"></div><div id=y style=\"height:10px\"></div></div>\
         </div>{C}",
        first(""),
        b("")
    );
    let (pages, boxes) = pages_of(&body);
    let got: Vec<(&str, usize, f32)> = boxes
        .iter()
        .filter(|(id, _)| id == "y" || id == "c")
        .map(|(id, (page, _, y, _, _))| (id.as_str(), *page, *y))
        .collect();
    assert_eq!(pages, 2, "page count, got {got:?}");
    assert_eq!(got, [("y", 1, 50.0), ("c", 1, 60.0)]);
}

/// An empty block that forces a break between the block before `b` and `b`
/// starts the second page, and the margin above `b` that adjoins that break
/// is kept, as when the block before is itself a place to break.
#[test]
fn a_forced_break_between_the_blocks_keeps_the_margin() {
    for (label, before) in [
        ("plain block before", first("")),
        ("unbreakable block before", first("break-inside:avoid")),
    ] {
        check(
            label,
            &format!(
                "<div>{before}<div style=\"break-before:page\"></div>{}</div>{C}",
                b("")
            ),
            &[("b", 1, 30.0, 50.0), ("c", 1, 50.0, 60.0)],
        );
    }
}

/// A block around `b` whose height does not shrink with its content keeps
/// its height when the margin is cut, and `c` after it stays below it; one
/// whose `min-height` is reached shrinks down to it.
#[test]
fn a_block_with_a_set_height_keeps_the_block_after_it_in_place() {
    for (label, wrapper, c_top) in [
        ("height", "height:150px", 50.0),
        ("min-height", "min-height:150px", 50.0),
        ("min-height reached", "min-height:125px", 25.0),
    ] {
        check(
            label,
            &format!("<div style=\"{wrapper}\">{}{}</div>{C}", first(""), b("")),
            &[("b", 1, 0.0, 20.0), ("c", 1, c_top, c_top + 10.0)],
        );
    }
    // Inside the block, `c` follows `b`.
    check(
        "c inside",
        &format!(
            "<div style=\"height:150px\">{}{}{C}</div>",
            first(""),
            b("")
        ),
        &[("b", 1, 0.0, 20.0), ("c", 1, 20.0, 30.0)],
    );
}
