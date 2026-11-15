//! An absolutely or fixed positioned box whose static position is in the
//! spaces where a line wraps takes it at the end of the line before the
//! break, where the content ends: collapsible spaces at the end of a line are
//! removed (CSS Text 3, 4.1.3), and the box stays with them on that line when
//! they fit on it and no inline box around it starts on the next line. A box
//! after the spaces at the end of a paragraph or before a forced break sits
//! where the content ends too.
//!
//! The expected rectangles are the ones Chromium gives the same markup in an
//! 800px by 600px viewport with a body margin of 8px, `div` boxes sized by
//! their border box and the Ahem font at 10px with a 10px line height. Each is
//! the left, top, width and height of the border box of an element with an
//! `id`.

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

/// The left, top, width and height of a box.
type Rect = (f32, f32, f32, f32);

const AHEM: &[u8] = include_bytes!("data/text-autospace/Ahem.ttf");

/// Twenty `x` and a space after each: four full lines of a 100px paragraph,
/// the last of which wraps at its last space.
const FOUR_LINES: &str = "x x x x x x x x x x x x x x x x x x x x ";

/// A positioned box with the identifier `id`.
fn positioned(id: &str) -> String {
    format!(r#"<span id={id} style="position:absolute;width:10px;height:10px"></span>"#)
}

/// The rectangle of each element with an `id` on the first page.
fn rects_of(body: &str) -> Vec<(String, Rect)> {
    let html = format!(
        "<!DOCTYPE html><html><head><style>@page{{size:800px 600px;margin:0}}\
         body{{margin:8px;font:10px/10px Ahem}}div{{box-sizing:border-box}}</style>\
         </head><body>{body}</body></html>"
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

/// Lay out `content` in a 100px wide positioned paragraph with the extra
/// `style` and compare the box of each `id` with `expected`.
fn check(style: &str, content: &str, expected: &[(&str, Rect)]) {
    check_body(
        &format!(r#"<div id=p style="width:100px;position:relative;{style}">{content}</div>"#),
        expected,
    );
}

/// Lay out `body` and compare the box of each `id` with `expected`.
fn check_body(body: &str, expected: &[(&str, Rect)]) {
    let rects = rects_of(body);
    for &(id, want) in expected {
        let got = rects
            .iter()
            .find(|(seen, _)| seen == id)
            .map(|(_, rect)| *rect)
            .unwrap_or_else(|| panic!("no box for #{id} in {rects:?}"));
        assert_eq!(got, want, "#{id} in {body}");
    }
}

#[test]
fn a_box_after_the_space_where_a_line_wraps_ends_that_line() {
    let s = positioned("s");
    let t = positioned("t");
    for content in [
        format!("{FOUR_LINES}{s}x x x x"),
        format!("{FOUR_LINES} {s}x x x x"),
        format!("{FOUR_LINES}{s} x x x x"),
        format!("x x x x x x x x x x x x x x x x x x x x<span> {s}x</span> x x x"),
        format!("x x x x x x x x x x x x x x x x x x x <span>x </span>{s}x x"),
        format!("x x x x x x x x x x x x x x x x x x x <b>x</b> {s}x x"),
        format!(r#"{FOUR_LINES}{s}<span style="padding-left:5px">x</span> x"#),
        format!(
            r#"{FOUR_LINES}{s}<span style="display:inline-block;width:10px;height:10px"></span>"#
        ),
        format!(
            r#"{FOUR_LINES}{s}<span style="display:inline-block;width:100px;height:10px"></span>"#
        ),
    ] {
        check("", &content, &[("s", (98.0, 38.0, 10.0, 10.0))]);
    }
    check(
        "",
        &format!("{FOUR_LINES}{s}{t}x x x x"),
        &[
            ("s", (98.0, 38.0, 10.0, 10.0)),
            ("t", (98.0, 38.0, 10.0, 10.0)),
        ],
    );
    check(
        "",
        &format!(
            r#"{FOUR_LINES}<span id=s style="position:fixed;width:10px;height:10px"></span>x x x x"#
        ),
        &[("s", (98.0, 38.0, 10.0, 10.0))],
    );
    check(
        "",
        &format!("x x x x x x x x x x {s}x x x x"),
        &[("s", (98.0, 18.0, 10.0, 10.0))],
    );
    let atomic = r#"<span style="display:inline-block;width:10px;height:10px"></span>"#;
    check(
        "",
        &format!("x x x x x x x x x x x x x x x x x x x {atomic} {s}x x"),
        &[("s", (98.0, 38.0, 10.0, 10.0))],
    );
    check(
        "",
        &format!("x x x x x x x x x x x x x x x x x x x {atomic} {s}"),
        &[("s", (98.0, 38.0, 10.0, 10.0))],
    );
    check(
        "hanging-punctuation:allow-end",
        &format!("x x x x x x x x x x x x x x x x x x xx. {s}x x"),
        &[("s", (98.0, 38.0, 10.0, 10.0))],
    );
    check(
        "white-space:pre-line",
        &format!("{FOUR_LINES}{s}x x x x"),
        &[("s", (98.0, 38.0, 10.0, 10.0))],
    );
}

#[test]
fn the_end_of_the_line_follows_its_alignment() {
    let s = positioned("s");
    for (align, x) in [("center", 103.0), ("right", 108.0), ("justify", 108.0)] {
        check(
            &format!("text-align:{align}"),
            &format!("{FOUR_LINES}{s}x x x x"),
            &[("s", (x, 38.0, 10.0, 10.0))],
        );
    }
}

/// Spaces that wrap instead of hanging are content of the line.
#[test]
fn a_box_after_spaces_that_break_stays_after_them() {
    check(
        "white-space:break-spaces",
        &format!("{FOUR_LINES}{}x x x x", positioned("s")),
        &[("s", (108.0, 38.0, 10.0, 10.0))],
    );
}

#[test]
fn a_box_after_the_spaces_at_the_end_of_a_paragraph_or_before_a_break_ends_the_content() {
    let s = positioned("s");
    check(
        "",
        &format!("{FOUR_LINES}{s}"),
        &[("s", (98.0, 38.0, 10.0, 10.0))],
    );
    check(
        "",
        &format!("{FOUR_LINES}{s}<br>x"),
        &[("s", (98.0, 38.0, 10.0, 10.0))],
    );
    check(
        "",
        &format!(
            r#"x x x x x x x x x x x x x x x x x <span style="padding-right:5px">x </span>{s}"#
        ),
        &[("s", (63.0, 38.0, 10.0, 10.0))],
    );
}

/// The box stays on the next line inside an inline box that starts there,
/// after spaces that do not fit, and when it was block-level, as it sits
/// below the line either way. A box in the middle of a line keeps its place.
#[test]
fn a_box_stays_on_the_next_line_when_the_line_before_cannot_take_it() {
    let s = positioned("s");
    for content in [
        format!("{FOUR_LINES}<span>{s}</span>x x x x"),
        format!("{FOUR_LINES}<span>{s}x</span> x"),
        format!(
            r#"x x x x x x x x x x x x x x x x x x x <span style="padding-right:5px">x </span>{s}x x"#
        ),
        format!(
            r#"x x x x x x x x x x x x x x x x x x x <span style="padding-right:5px">x</span> {s}x x"#
        ),
        format!(
            r#"{FOUR_LINES}<span id=s style="position:absolute;display:block;width:10px;height:10px"></span>x x x x"#
        ),
    ] {
        check("", &content, &[("s", (8.0, 48.0, 10.0, 10.0))]);
    }
    check(
        "",
        &format!(r#"{FOUR_LINES}<span style="padding-left:5px">{s}x</span>"#),
        &[("s", (13.0, 48.0, 10.0, 10.0))],
    );
    check(
        "",
        &format!("x x x x x x x x x x x x x x x x x x x {s} x x x x"),
        &[("s", (88.0, 38.0, 10.0, 10.0))],
    );
    check(
        "direction:rtl",
        &format!("x x x x {s}x x x x"),
        &[("s", (98.0, 8.0, 10.0, 10.0))],
    );
}

/// A float after the boxes leaves no room for them at the end of the line,
/// so they start the next one.
#[test]
fn boxes_before_a_float_start_the_next_line() {
    check(
        "",
        &format!(
            r#"{FOUR_LINES}{}{}<span id=f style="float:right;width:10px;height:10px"></span>x x"#,
            positioned("s"),
            positioned("t")
        ),
        &[
            ("s", (8.0, 48.0, 10.0, 10.0)),
            ("t", (8.0, 48.0, 10.0, 10.0)),
        ],
    );
}

/// A block-level box after the spaces sits below the line in the column
/// where the line breaker put it.
#[test]
fn a_block_level_box_in_columns_stays_in_the_column_of_the_next_line() {
    check_body(
        &format!(
            r#"<div id=m style="columns:2;column-gap:0;width:200px;position:relative"><div id=p>{FOUR_LINES}<span id=s style="position:absolute;display:block;width:10px;height:10px"></span>x x x x</div></div>"#
        ),
        &[("s", (108.0, 18.0, 10.0, 10.0))],
    );
}

#[test]
fn a_box_in_columns_goes_with_the_end_of_its_line() {
    check_body(
        &format!(
            r#"<div id=m style="columns:2;column-gap:0;width:200px;position:relative"><div id=p>{FOUR_LINES}{}x x x x</div></div>"#,
            positioned("s")
        ),
        &[("s", (198.0, 8.0, 10.0, 10.0))],
    );
}

/// A tab that stays on the line keeps its advance before the box.
#[test]
fn a_box_after_a_kept_tab_stays_at_the_tab_stop() {
    let s = positioned("s");
    for content in [format!("x\t{s}"), format!("x\t{s}y")] {
        check_body(
            &format!(
                r#"<div id=p style="width:300px;position:relative;white-space:pre;tab-size:4">{content}</div>"#
            ),
            &[("s", (48.0, 8.0, 10.0, 10.0))],
        );
    }
}
