//! A non-atomic inline box keeps the rules of an inline box, whether it holds
//! text, a block that splits it (CSS 2.1 section 9.2.1.1) or nothing: its `width`,
//! `height`, `min-*` and `max-*` sizes and its vertical margins do not apply to
//! it (sections 8.3, 10.3.1 and 10.6.1), and its vertical padding and border are
//! painted without changing the height of a line. The block inside takes its
//! width from the block that contains the inline box, whatever the inline box
//! says. An edge along the line (a side border, say) still makes a line box
//! before and after the block, and a line of its own for a box with nothing in
//! it.
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
fn an_absolute_box_in_a_relative_inline_box_keeps_the_padding_box_with_its_vertical_border() {
    // The padding box of the inline box is its border box less the border it
    // has, although the border takes no part in the height of the line, and
    // the out-of-flow box takes no room on the line after the block.
    check(
        r#"<div id=o style="width:100px"><span style="position:relative;border-top:6px solid;padding-top:4px">x<div id=b style="height:10px"></div><div id=a style="position:absolute;top:0;left:0;width:10px;height:10px"></div></span></div>"#,
        &[
            ("o", (8.0, 8.0, 100.0, 20.0)),
            ("b", (8.0, 18.0, 100.0, 10.0)),
            ("a", (8.0, 4.0, 10.0, 10.0)),
        ],
    );
    check(
        r#"<div id=o style="width:100px"><span style="position:relative;border-bottom:6px solid;padding-bottom:4px">x<div id=b style="height:10px"></div><div id=a style="position:absolute;bottom:0;left:0;width:10px;height:10px"></div></span></div>"#,
        &[
            ("o", (8.0, 8.0, 100.0, 20.0)),
            ("b", (8.0, 18.0, 100.0, 10.0)),
            ("a", (8.0, 12.0, 10.0, 10.0)),
        ],
    );
}

#[test]
fn a_line_of_text_without_a_visible_glyph_keeps_its_line() {
    // A soft hyphen that is not shown is still text on its line.
    check(
        r#"<div id=o style="width:100px"><span>&shy;</span></div><div id=c style="height:10px"></div>"#,
        &[
            ("o", (8.0, 8.0, 100.0, 10.0)),
            ("c", (8.0, 18.0, 784.0, 10.0)),
        ],
    );
    check(
        r#"<div id=o style="width:100px">x<br><span>&shy;</span></div><div id=c style="height:10px"></div>"#,
        &[
            ("o", (8.0, 8.0, 100.0, 20.0)),
            ("c", (8.0, 28.0, 784.0, 10.0)),
        ],
    );
    check(
        r#"<div id=o style="width:100px"><span>x<div id=b style="height:10px"></div>&shy;</span></div><div id=c style="height:10px"></div>"#,
        &[
            ("o", (8.0, 8.0, 100.0, 30.0)),
            ("b", (8.0, 18.0, 100.0, 10.0)),
            ("c", (8.0, 38.0, 784.0, 10.0)),
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
    // An edge along the line counts on its own: a negative margin on the
    // other side leaves the line as wide as nothing, and it is still a line.
    for style in [
        "padding-left:5px;margin-right:-5px",
        "margin-left:5px;margin-right:-5px",
        "border-right:5px solid;margin-left:-5px",
    ] {
        check(
            &format!(
                r#"<div id=o style="width:100px">x<br><span style="{style}"></span></div><div id=c style="height:10px"></div>"#
            ),
            &[
                ("o", (8.0, 8.0, 100.0, 20.0)),
                ("c", (8.0, 28.0, 784.0, 10.0)),
            ],
        );
    }
    // A line that ends in a break is a line whatever its boxes hold.
    check(
        r#"<div id=o style="width:100px">x<br><span style="padding-top:20px"></span><br>y</div><div id=c style="height:10px"></div>"#,
        &[
            ("o", (8.0, 8.0, 100.0, 30.0)),
            ("c", (8.0, 38.0, 784.0, 10.0)),
        ],
    );
}

/// `inner` in a container 100px wide, followed by a block of 10px.
fn in_container(inner: &str, container_style: &str) -> String {
    format!(
        r#"<div id=o style="width:100px;{container_style}">{inner}</div><div id=c style="height:10px"></div>"#
    )
}

/// An inline box with `style` and no content in a container of its own.
fn around_nothing(style: &str) -> String {
    in_container(&format!(r#"<span style="{style}"></span>"#), "")
}

/// The container `o` is `o` high and the block `c` after it starts at `8 + o`.
fn container_height(o: f32) -> [(&'static str, Rect); 2] {
    [
        ("o", (8.0, 8.0, 100.0, o)),
        ("c", (8.0, 8.0 + o, 784.0, 10.0)),
    ]
}

#[test]
fn an_empty_inline_box_ignores_what_does_not_apply_to_it() {
    for style in IGNORED.iter().chain(&[
        "width:30px",
        "height:20px",
        "width:30px;height:20px",
        "position:relative;top:3px;padding:20px 0",
        "position:relative;width:30px;height:20px",
    ]) {
        check(&around_nothing(style), &container_height(0.0));
    }
}

#[test]
fn an_empty_inline_box_with_an_edge_along_the_line_makes_a_line() {
    for style in [
        "border-left:5px solid",
        "padding-left:5px",
        "border:5px solid",
        "padding:20px",
        "padding:20px 0 20px 5px",
    ] {
        check(&around_nothing(style), &container_height(10.0));
    }
}

#[test]
fn an_inline_box_holding_nothing_but_whitespace_or_nothing_drawn_is_empty() {
    let style = "padding:20px 0";
    for inner in [
        format!(r#"<span style="{style}"> </span>"#),
        format!(r#" <span style="{style}"></span>"#),
        format!(r#"<span style="{style}"></span> "#),
        format!(r#"<span style="{style}"><!-- c --></span>"#),
        format!(r#"<span style="{style}"><div style="display:none"></div></span>"#),
        format!(r#"<a style="{style}"></a>"#),
    ] {
        check(&in_container(&inner, ""), &container_height(0.0));
    }
}

#[test]
fn empty_inline_boxes_nested_or_in_a_row_ignore_what_does_not_apply_to_them() {
    let style = "padding:20px 0";
    for inner in [
        format!(r#"<span style="{style}"><span style="{style}"></span></span>"#),
        format!(r#"<span style="{style}"><span></span></span>"#),
        format!(r#"<span style="{style}"></span><span style="{style}"></span>"#),
        format!(r#"<span><span style="{style}"></span></span>"#),
    ] {
        check(&in_container(&inner, ""), &container_height(0.0));
    }
}

#[test]
fn an_empty_inline_box_next_to_text_or_blocks_makes_no_line_of_its_own() {
    let span = r#"<span style="padding:20px 0"></span>"#;
    let block = |id| format!(r#"<div id={id} style="height:10px"></div>"#);
    for inner in [format!("x{span}"), format!("{span}x")] {
        check(&in_container(&inner, ""), &container_height(10.0));
    }
    check(
        &in_container(&format!("{}{span}", block("b")), ""),
        &[
            ("o", (8.0, 8.0, 100.0, 10.0)),
            ("b", (8.0, 8.0, 100.0, 10.0)),
            ("c", (8.0, 18.0, 784.0, 10.0)),
        ],
    );
    check(
        &in_container(&format!("{span}{}", block("b")), ""),
        &[
            ("o", (8.0, 8.0, 100.0, 10.0)),
            ("b", (8.0, 8.0, 100.0, 10.0)),
            ("c", (8.0, 18.0, 784.0, 10.0)),
        ],
    );
    check(
        &in_container(&format!("{}{span}{}", block("b"), block("b2")), ""),
        &[
            ("o", (8.0, 8.0, 100.0, 20.0)),
            ("b", (8.0, 8.0, 100.0, 10.0)),
            ("b2", (8.0, 18.0, 100.0, 10.0)),
            ("c", (8.0, 28.0, 784.0, 10.0)),
        ],
    );
}

#[test]
fn an_empty_inline_box_in_any_container_ignores_what_does_not_apply_to_it() {
    let span = r#"<span style="padding:20px 0"></span>"#;
    check(
        &format!(
            r#"<div id=o style="display:inline-block;vertical-align:top;width:100px">{span}</div>"#
        ),
        &[("o", (8.0, 8.0, 100.0, 0.0))],
    );
    check(
        &format!(
            r#"<ul style="margin:0;padding:0;list-style:none"><li id=o>{span}</li></ul><div id=c style="height:10px"></div>"#
        ),
        &[
            ("o", (8.0, 8.0, 784.0, 0.0)),
            ("c", (8.0, 8.0, 784.0, 10.0)),
        ],
    );
    check(
        &format!(
            r#"<div style="display:table"><div style="display:table-row"><div id=o style="display:table-cell;width:100px">{span}</div></div></div><div id=c style="height:10px"></div>"#
        ),
        &[
            ("o", (8.0, 8.0, 100.0, 0.0)),
            ("c", (8.0, 8.0, 784.0, 10.0)),
        ],
    );
    check(
        &in_container(&format!("<span>{span}</span>"), ""),
        &container_height(0.0),
    );
}

#[test]
fn an_empty_box_that_a_flex_or_grid_container_blockifies_keeps_what_applies_to_it() {
    for container in ["display:flex", "display:grid"] {
        check(
            &in_container(r#"<span style="padding:20px 0"></span>"#, container),
            &container_height(40.0),
        );
    }
    check(
        &in_container(
            r#"<span style="width:30px;height:20px"></span>"#,
            "display:flex",
        ),
        &container_height(20.0),
    );
}

#[test]
fn an_empty_box_that_floats_or_is_absolute_takes_no_room_in_the_flow() {
    for style in [
        "float:left;width:30px;height:20px",
        "position:absolute;width:30px;height:20px",
    ] {
        check(&around_nothing(style), &container_height(0.0));
    }
}

#[test]
fn an_empty_inline_box_before_a_break_or_holding_one_still_makes_a_line() {
    check(&in_container("<br>", ""), &container_height(10.0));
    check(
        &in_container(r#"<span style="padding:20px 0"><br></span>"#, ""),
        &container_height(10.0),
    );
}

#[test]
fn an_inline_box_with_text_keeps_the_box_it_is_painted_with() {
    // The vertical padding and border stay in the box of an inline box with
    // text, though they take no room on the line; its sizes do not apply.
    let around = |style: &str, text: &str| {
        format!(
            r#"<div id=o style="margin-top:100px;width:100px">{text}<span id=s style="{style}">yy</span></div><div id=c style="height:10px"></div>"#
        )
    };
    let rects = |s: Rect| [("o", (8.0, 100.0, 100.0, 10.0)), ("s", s)];
    check(
        &around("padding:20px 0", "x"),
        &rects((18.0, 80.0, 20.0, 50.0)),
    );
    check(
        &around("border-top:5px solid;border-bottom:7px solid", "x"),
        &rects((18.0, 95.0, 20.0, 22.0)),
    );
    check(
        &around("width:50px;height:50px;padding:2px", "x"),
        &rects((18.0, 98.0, 24.0, 14.0)),
    );
    check(
        &around("padding:20px 0", ""),
        &rects((8.0, 80.0, 20.0, 50.0)),
    );
}
