//! An inline box that holds a block (CSS 2.1 section 9.2.1.1) keeps the rules of
//! an inline box: its `width`, `height`, `min-*` and `max-*` sizes and its
//! vertical margins do not apply to it (sections 8.3, 10.3.1 and 10.6.1), and
//! its vertical padding and border are painted without changing the height of
//! a line. The block inside takes its width from the block that contains the
//! inline box, whatever the inline box says. An edge along the line (a side
//! border, say) still makes a line box before and after the block.
//!
//! The expected border boxes are the ones Chromium computes for the same
//! markup in an 800px by 600px viewport, with the body margin of 8px.

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

const AHEM: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/data/text-autospace/Ahem.ttf"
));

/// A border box `(x, y, width, height)`.
type Rect = (f32, f32, f32, f32);

/// The border boxes of the elements of `body` with an `id` on the first page.
fn border_boxes(body: &str) -> Vec<(String, Rect)> {
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
        .filter_map(|fragment| {
            let id = dom.attr(fragment.node(), "id")?;
            let rect = fragment.rect();
            Some((id.to_owned(), (rect.x, rect.y, rect.width, rect.height)))
        })
        .collect()
}

/// Layout units are 1/64px; allow a little more for the 32-bit sums.
const TOLERANCE: f32 = 0.02;

fn check(body: &str, expected: &[(&str, Rect)]) {
    let actual = border_boxes(body);
    for &(id, (x, y, width, height)) in expected {
        let found = actual
            .iter()
            .find(|(name, _)| name == id)
            .unwrap_or_else(|| panic!("no fragment for #{id} in {body}"))
            .1;
        let close = |a: f32, b: f32| (a - b).abs() <= TOLERANCE;
        assert!(
            close(found.0, x)
                && close(found.1, y)
                && close(found.2, width)
                && close(found.3, height),
            "{body}: #{id} is {found:?}, expected {:?}",
            (x, y, width, height)
        );
    }
}

/// Styles of the inline box that change nothing about the layout.
const IGNORED: &[&str] = &[
    "",
    "width:50px",
    "height:50px",
    "min-height:50px",
    "max-height:5px",
    "min-width:50px",
    "max-width:50px",
    "margin-top:20px",
    "margin-bottom:20px",
    "margin:20px 0",
    "padding-top:20px",
    "padding-bottom:20px",
    "padding:20px 0",
    "border-top:5px solid",
    "border-bottom:5px solid",
];

/// `style` on an inline box that holds `text` and a block of 10px, followed
/// by a block of 10px in the same 100px wide container.
fn around_block(style: &str, text: &str) -> String {
    format!(
        r#"<div id=o style="width:100px"><span style="{style}">{text}<div id=b style="height:10px"></div></span><div id=c style="height:10px"></div></div>"#
    )
}

#[test]
fn an_inline_box_around_only_a_block_ignores_what_does_not_apply_to_it() {
    for style in IGNORED {
        check(
            &around_block(style, ""),
            &[
                ("o", (8.0, 8.0, 100.0, 20.0)),
                ("b", (8.0, 8.0, 100.0, 10.0)),
                ("c", (8.0, 18.0, 100.0, 10.0)),
            ],
        );
    }
}

#[test]
fn an_inline_box_with_text_before_a_block_ignores_what_does_not_apply_to_it() {
    for style in IGNORED {
        check(
            &around_block(style, "x"),
            &[
                ("o", (8.0, 8.0, 100.0, 30.0)),
                ("b", (8.0, 18.0, 100.0, 10.0)),
                ("c", (8.0, 28.0, 100.0, 10.0)),
            ],
        );
    }
}

#[test]
fn a_side_border_makes_a_line_before_and_after_the_block() {
    check(
        &around_block("border:5px solid", ""),
        &[
            ("o", (8.0, 8.0, 100.0, 40.0)),
            ("b", (8.0, 18.0, 100.0, 10.0)),
            ("c", (8.0, 38.0, 100.0, 10.0)),
        ],
    );
    check(
        &around_block("border:5px solid", "x"),
        &[
            ("o", (8.0, 8.0, 100.0, 40.0)),
            ("b", (8.0, 18.0, 100.0, 10.0)),
            ("c", (8.0, 38.0, 100.0, 10.0)),
        ],
    );
}

#[test]
fn an_edge_along_the_line_makes_a_line_on_its_side_of_the_block() {
    let rects = |o: f32, b: f32, c: f32| {
        [
            ("o", (8.0, 8.0, 100.0, o)),
            ("b", (8.0, b, 100.0, 10.0)),
            ("c", (8.0, c, 100.0, 10.0)),
        ]
    };
    // The start edge keeps the line before the block, the end edge the one
    // after it; text before the block already makes the first.
    check(
        &around_block("padding-left:5px", ""),
        &rects(30.0, 18.0, 28.0),
    );
    check(
        &around_block("margin-left:5px", ""),
        &rects(30.0, 18.0, 28.0),
    );
    check(
        &around_block("padding-right:5px", ""),
        &rects(30.0, 8.0, 28.0),
    );
    check(
        &around_block("padding-left:5px", "x"),
        &rects(30.0, 18.0, 28.0),
    );
    check(
        &around_block("padding-right:5px", "x"),
        &rects(40.0, 18.0, 38.0),
    );
}

#[test]
fn inline_boxes_nested_around_a_block_ignore_what_does_not_apply_to_them() {
    let rects = |o: f32, b: (f32, f32, f32), c: f32| {
        [
            ("o", (8.0, 8.0, 100.0, o)),
            ("b", (8.0, b.0, b.1, b.2)),
            ("c", (8.0, c, 100.0, 10.0)),
        ]
    };
    // Text before the block, in the outer box.
    check(
        r#"<div id=o style="width:100px"><span style="width:50px;padding-top:20px;margin-bottom:9px">x<span style="height:40px;margin-top:10px;padding-bottom:7px"><div id=b style="height:10px"></div></span></span><div id=c style="height:10px"></div></div>"#,
        &rects(30.0, (18.0, 100.0, 10.0), 28.0),
    );
    // Nothing but the block.
    check(
        r#"<div id=o style="width:100px"><span style="width:50px;padding-top:20px;margin-bottom:9px"><span style="height:40px;margin-top:10px;padding-bottom:7px"><div id=b style="height:10px"></div></span></span><div id=c style="height:10px"></div></div>"#,
        &rects(20.0, (8.0, 100.0, 10.0), 18.0),
    );
    // Text after the block, in the outer box.
    check(
        r#"<div id=o style="width:100px"><span style="height:40px;margin-top:10px"><b style="width:60px;padding-top:6px"><div id=b style="height:10px"></div></b>y</span><div id=c style="height:10px"></div></div>"#,
        &rects(30.0, (8.0, 100.0, 10.0), 28.0),
    );
}

#[test]
fn a_percentage_width_in_an_inline_box_resolves_against_the_block_around_it() {
    check(
        r#"<div id=o style="width:100px"><span style="width:50px;height:40px;margin-top:10px">x<div id=b style="width:50%;height:10px"></div>y</span><div id=c style="height:10px"></div></div>"#,
        &[
            ("o", (8.0, 8.0, 100.0, 40.0)),
            ("b", (8.0, 18.0, 50.0, 10.0)),
            ("c", (8.0, 38.0, 100.0, 10.0)),
        ],
    );
    // A float in the inline box is not a block that splits it.
    check(
        r#"<div id=o style="width:100px"><span style="width:50px;height:30px;min-height:30px">x<div id=b style="height:10px;float:left;width:20px"></div>y</span><div id=c style="height:10px"></div></div>"#,
        &[
            ("o", (8.0, 8.0, 100.0, 20.0)),
            ("b", (8.0, 8.0, 20.0, 10.0)),
            ("c", (8.0, 18.0, 100.0, 10.0)),
        ],
    );
}

#[test]
fn a_last_line_of_inline_boxes_with_only_vertical_edges_makes_no_line() {
    for style in [
        "padding-top:20px",
        "padding-bottom:20px",
        "border-top:5px solid",
        "margin-top:20px",
    ] {
        check(
            &format!(
                r#"<div id=o style="width:100px">x<br><span style="{style}"></span></div><div id=c style="height:10px"></div>"#
            ),
            &[
                ("o", (8.0, 8.0, 100.0, 10.0)),
                ("c", (8.0, 18.0, 784.0, 10.0)),
            ],
        );
    }
    // An edge along the line keeps a box of the height of a line.
    check(
        r#"<div id=o style="width:100px"><span style="padding-left:5px"></span></div><div id=c style="height:10px"></div>"#,
        &[
            ("o", (8.0, 8.0, 100.0, 10.0)),
            ("c", (8.0, 18.0, 784.0, 10.0)),
        ],
    );
    // A line that ends in a break is a line whatever its boxes hold.
    check(
        r#"<div id=o style="width:100px">x<br><span style="padding-top:20px"></span><br>y</div><div id=c style="height:10px"></div>"#,
        &[
            ("o", (8.0, 8.0, 100.0, 30.0)),
            ("c", (8.0, 38.0, 784.0, 10.0)),
        ],
    );
}
