//! A table with no rows and no cells is as large as its `min-width`,
//! `max-width`, `min-height` and `max-height` say, like a table that has rows:
//! the minimum grows it and the maximum caps a size it states, while a maximum
//! below the padding and border still leaves the table as large as those.
//!
//! The expected boxes are the ones Chromium computes for the same markup in an
//! 800px by 600px viewport, with the body margin of 8px and `box-sizing:
//! border-box` on every `div` unless a case says otherwise: each is the left,
//! top, width and height of the element.

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

fn assert_box(label: &str, body: &str, id: &str, expected: [f32; 4]) {
    let found = border_box(body, id);
    assert!(
        found
            .iter()
            .zip(expected)
            .all(|(got, want)| (got - want).abs() <= TOLERANCE),
        "{label}: #{id} is {found:?}, expected {expected:?} (left, top, width, height)"
    );
}

/// The markup of an empty table `#o` with the given style.
fn empty_table(style: &str) -> String {
    format!("<div id=o style=\"display:table;{style}\"></div>")
}

/// Checks an empty table with the given style, the label being the style.
fn check(cases: &[(&str, [f32; 4])]) {
    for (style, expected) in cases {
        assert_box(style, &empty_table(style), "o", *expected);
    }
}

#[test]
fn min_width_grows_an_empty_table() {
    check(&[
        ("min-width:100px", [8.0, 8.0, 100.0, 0.0]),
        // The padding is inside the width of a border-box table.
        ("min-width:100px;padding:10px", [8.0, 8.0, 100.0, 20.0]),
        // And outside the width of a content-box one.
        (
            "min-width:100px;padding:10px;box-sizing:content-box",
            [8.0, 8.0, 120.0, 20.0],
        ),
        // A minimum above the width the table states wins.
        ("width:80px;min-width:100px", [8.0, 8.0, 100.0, 0.0]),
        // A minimum above the maximum wins as well.
        ("min-width:100px;max-width:50px", [8.0, 8.0, 100.0, 0.0]),
    ]);
}

#[test]
fn max_width_caps_the_width_an_empty_table_states() {
    check(&[
        ("width:20px;max-width:10px", [8.0, 8.0, 10.0, 0.0]),
        (
            "width:50px;max-width:30px;padding:10px",
            [8.0, 8.0, 30.0, 20.0],
        ),
        (
            "width:50px;max-width:30px;padding:10px;box-sizing:content-box",
            [8.0, 8.0, 50.0, 20.0],
        ),
    ]);
}

#[test]
fn max_width_never_shrinks_an_empty_table_below_its_padding_and_border() {
    check(&[
        (
            "padding:10px;border:2px solid;max-width:5px",
            [8.0, 8.0, 24.0, 24.0],
        ),
        // The maximum does not grow a table that states no width.
        ("max-width:30px", [8.0, 8.0, 0.0, 0.0]),
    ]);
}

#[test]
fn percentages_of_an_empty_table_resolve_against_its_container() {
    let in_container = |style: &str| {
        format!("<div style=\"width:200px\"><div id=o style=\"display:table;{style}\"></div></div>")
    };
    assert_box(
        "min-width:50%",
        &in_container("min-width:50%"),
        "o",
        [8.0, 8.0, 100.0, 0.0],
    );
    assert_box(
        "width:50%;max-width:20%",
        &in_container("width:50%;max-width:20%"),
        "o",
        [8.0, 8.0, 40.0, 0.0],
    );
}

#[test]
fn min_width_grows_an_empty_table_in_every_context() {
    // The label, the style, and where Chromium puts the table: the bottom edge
    // of an empty inline-table is on the baseline, 8px below the line's top.
    let cases = [
        ("an inline-table", "display:inline-table", [8.0, 16.0]),
        ("a float", "display:table;float:left", [8.0, 8.0]),
        (
            "absolutely positioned",
            "display:table;position:absolute",
            [8.0, 8.0],
        ),
        (
            "a vertical table",
            "display:table;writing-mode:vertical-rl",
            [8.0, 8.0],
        ),
    ];
    for (label, style, [left, top]) in cases {
        let body = format!("<div id=o style=\"{style};min-width:100px\"></div>");
        assert_box(label, &body, "o", [left, top, 100.0, 0.0]);
    }
    assert_box(
        "a flex item",
        "<div style=\"display:flex\"><div id=o style=\"display:table;min-width:100px\"></div></div>",
        "o",
        [8.0, 8.0, 100.0, 0.0],
    );
}

#[test]
fn a_caption_is_as_wide_as_the_min_width_of_the_empty_table_around_it() {
    let body = "<div id=o style=\"display:table;min-width:100px\">\
                <div id=c style=\"display:table-caption\">aa bbb cc</div></div>";
    assert_box("the caption", body, "c", [8.0, 8.0, 100.0, 10.0]);
}

#[test]
fn min_height_and_max_height_size_an_empty_table_the_same_way() {
    check(&[
        ("min-height:50px", [8.0, 8.0, 0.0, 50.0]),
        ("height:30px;max-height:10px", [8.0, 8.0, 0.0, 10.0]),
        ("min-height:50px;padding:10px", [8.0, 8.0, 20.0, 50.0]),
        (
            "min-height:50px;padding:10px;box-sizing:content-box",
            [8.0, 8.0, 20.0, 70.0],
        ),
        (
            "height:30px;max-height:10px;padding:10px;box-sizing:content-box",
            [8.0, 8.0, 20.0, 30.0],
        ),
        ("max-height:10px;padding:10px", [8.0, 8.0, 20.0, 20.0]),
        ("min-height:50px;max-height:20px", [8.0, 8.0, 0.0, 50.0]),
        (
            "min-height:50px;writing-mode:vertical-rl",
            [8.0, 8.0, 0.0, 50.0],
        ),
    ]);
}

#[test]
fn a_stated_size_of_a_content_box_empty_table_is_outside_its_padding_and_border() {
    check(&[
        (
            "width:20px;padding:10px;box-sizing:content-box",
            [8.0, 8.0, 40.0, 20.0],
        ),
        (
            "height:20px;padding:10px;box-sizing:content-box",
            [8.0, 8.0, 20.0, 40.0],
        ),
        (
            "height:5px;padding:10px;box-sizing:content-box",
            [8.0, 8.0, 20.0, 25.0],
        ),
        // The maximum is outside the padding too, and above the size.
        (
            "width:20px;max-width:30px;padding:10px;box-sizing:content-box",
            [8.0, 8.0, 40.0, 20.0],
        ),
        (
            "height:20px;max-height:30px;padding:10px;box-sizing:content-box",
            [8.0, 8.0, 20.0, 40.0],
        ),
        (
            "width:20px;min-width:10px;padding:10px;box-sizing:content-box",
            [8.0, 8.0, 40.0, 20.0],
        ),
    ]);
    // The same in a box that shrinks to fit its content.
    let body = "<div style=\"display:inline-block\">\
                <div id=o style=\"display:table;width:20px;padding:10px;box-sizing:content-box\"></div></div>";
    assert_box("in an inline-block", body, "o", [8.0, 8.0, 40.0, 20.0]);
}

#[test]
fn a_max_below_the_padding_and_border_leaves_a_stated_empty_table_as_large_as_they_are() {
    check(&[
        (
            "width:50px;max-width:3px;border:4px solid",
            [8.0, 8.0, 8.0, 8.0],
        ),
        ("padding:4px;width:50px;max-width:3px", [8.0, 8.0, 8.0, 8.0]),
        (
            "height:50px;max-height:3px;border:4px solid",
            [8.0, 8.0, 8.0, 8.0],
        ),
        // The maximums are outside the padding of a content-box table.
        (
            "width:20px;height:20px;padding:10px;box-sizing:content-box;max-width:25px;max-height:25px",
            [8.0, 8.0, 40.0, 40.0],
        ),
    ]);
}

#[test]
fn the_box_that_shrinks_around_an_empty_table_is_as_large_as_the_table() {
    for (label, wrapper) in [
        ("an inline-block", "display:inline-block"),
        ("a float", "float:left"),
        ("absolutely positioned", "position:absolute"),
    ] {
        let body = format!(
            "<div id=w style=\"{wrapper}\"><div id=o style=\"display:table;width:20px;\
             padding:10px;box-sizing:content-box\"></div></div>"
        );
        assert_box(label, &body, "o", [8.0, 8.0, 40.0, 20.0]);
        assert_box(label, &body, "w", [8.0, 8.0, 40.0, 20.0]);
    }
}

#[test]
fn an_empty_table_item_of_a_flex_or_grid_container_sizes_like_one_in_a_block() {
    let item = |container: &str, style: &str| {
        format!("<div style=\"{container}\"><div id=o style=\"display:table;{style}\"></div></div>")
    };
    let cases = [
        (
            "flex row",
            "display:flex",
            "width:50px;max-width:3px;border:4px solid",
            [8.0, 8.0, 8.0, 8.0],
        ),
        (
            "flex row",
            "display:flex",
            "width:20px;padding:10px;box-sizing:content-box",
            [8.0, 8.0, 40.0, 20.0],
        ),
        // A column stretches its items, up to their maximum.
        (
            "flex column",
            "display:flex;flex-direction:column;width:100px",
            "max-width:30px",
            [8.0, 8.0, 30.0, 0.0],
        ),
        (
            "flex column",
            "display:flex;flex-direction:column;width:100px",
            "min-width:50px",
            [8.0, 8.0, 100.0, 0.0],
        ),
        (
            "grid",
            "display:grid",
            "width:20px;max-width:10px",
            [8.0, 8.0, 10.0, 0.0],
        ),
        (
            "grid",
            "display:grid",
            "width:20px;height:20px;padding:10px;box-sizing:content-box;max-width:25px;max-height:25px",
            [8.0, 8.0, 40.0, 40.0],
        ),
    ];
    for (label, container, style, expected) in cases {
        assert_box(label, &item(container, style), "o", expected);
    }
}
