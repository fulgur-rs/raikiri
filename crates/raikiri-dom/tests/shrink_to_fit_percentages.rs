//! A box that shrinks to fit its content takes the size of the boxes inside it
//! as they are without the percentages that refer to its own width: a
//! percentage `width`, `min-width` or `max-width` of such a box counts as
//! `auto` and as no limit, and a percentage margin or padding counts as zero
//! (CSS Sizing 3 §5.2.1). The percentages then resolve against the width the
//! container ends up with.
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

/// The containers whose width is decided by their content.
const CONTAINERS: [(&str, &str); 5] = [
    ("a float", "<div id=r style=\"float:left\">{}</div>"),
    (
        "an inline-block",
        "<div id=r style=\"display:inline-block\">{}</div>",
    ),
    (
        "an absolutely positioned box",
        "<div id=r style=\"position:absolute\">{}</div>",
    ),
    (
        "a table cell",
        "<div style=\"display:table\"><div id=r style=\"display:table-cell\">{}</div></div>",
    ),
    (
        "a flex item",
        "<div style=\"display:flex\"><div id=r>{}</div></div>",
    ),
];

/// Checks `content` in each of the shrink-to-fit containers against
/// `expected`.
fn check_in_each_container(content: &str, expected: &[(&str, Rect)]) {
    for (_, container) in CONTAINERS {
        check(&container.replace("{}", content), expected);
    }
}

#[test]
fn a_percentage_width_of_an_atomic_inline_counts_as_auto_for_its_shrink_to_fit_container() {
    // The container takes the 20px of `aa` and the 40px of `bbbb`, and the
    // atomic then takes half of the 60px.
    check_in_each_container(
        "aa<div id=a style=\"display:inline-block;width:50%\">bbbb</div>",
        &[
            ("r", (8.0, 8.0, 60.0, 10.0)),
            ("a", (28.0, 8.0, 30.0, 10.0)),
        ],
    );
}

#[test]
fn a_percentage_width_of_a_float_counts_as_auto_for_its_shrink_to_fit_container() {
    check_in_each_container(
        "aa<div id=f style=\"float:left;width:50%\">bbbb</div>",
        &[("r", (8.0, 8.0, 60.0, 10.0)), ("f", (8.0, 8.0, 30.0, 10.0))],
    );
}

#[test]
fn a_percentage_width_of_a_block_counts_as_auto_for_its_shrink_to_fit_container() {
    // The block follows the line of `aa`; the container is as wide as the
    // block's 40px.
    check_in_each_container(
        "aa<div id=a style=\"display:block;width:50%\">bbbb</div>",
        &[
            ("r", (8.0, 8.0, 40.0, 20.0)),
            ("a", (8.0, 18.0, 20.0, 10.0)),
        ],
    );
}

#[test]
fn a_percentage_min_or_max_width_does_not_limit_the_shrink_to_fit_container() {
    for (style, atomic, float) in [
        // `min-width` brings the box up to half of the container only when
        // its content is narrower.
        (
            "min-width:50%",
            ("a", (28.0, 8.0, 40.0, 10.0)),
            ("f", (8.0, 8.0, 40.0, 10.0)),
        ),
        (
            "max-width:50%",
            ("a", (28.0, 8.0, 30.0, 10.0)),
            ("f", (8.0, 8.0, 30.0, 10.0)),
        ),
    ] {
        check_in_each_container(
            &format!("aa<div id=a style=\"display:inline-block;{style}\">bbbb</div>"),
            &[("r", (8.0, 8.0, 60.0, 10.0)), atomic],
        );
        check_in_each_container(
            &format!("aa<div id=f style=\"float:left;{style}\">bbbb</div>"),
            &[("r", (8.0, 8.0, 60.0, 10.0)), float],
        );
    }
}

#[test]
fn a_calc_width_with_a_percentage_counts_as_auto_for_its_shrink_to_fit_container() {
    check_in_each_container(
        "aa<div id=a style=\"display:inline-block;width:calc(50% + 10px)\">bbbb</div>",
        &[
            ("r", (8.0, 8.0, 60.0, 10.0)),
            ("a", (28.0, 8.0, 40.0, 10.0)),
        ],
    );
    check_in_each_container(
        "aa<div id=f style=\"float:left;width:calc(50% + 10px)\">bbbb</div>",
        &[("r", (8.0, 8.0, 60.0, 10.0)), ("f", (8.0, 8.0, 40.0, 10.0))],
    );
}

#[test]
fn the_padding_and_border_of_a_percentage_width_box_stay_in_the_container_width() {
    // The atomic takes its 40px of text and 20px of padding as its
    // contribution; its border box then takes half of the 80px.
    check_in_each_container(
        "aa<div id=a style=\"display:inline-block;width:50%;padding:0 10px\">bbbb</div>",
        &[
            ("r", (8.0, 8.0, 80.0, 10.0)),
            ("a", (28.0, 8.0, 40.0, 10.0)),
        ],
    );
    check_in_each_container(
        "aa<div id=a style=\"display:inline-block;width:50%;padding:0 10px;border:2px solid\">bbbb</div>",
        &[
            ("r", (8.0, 8.0, 84.0, 14.0)),
            ("a", (28.0, 8.0, 42.0, 14.0)),
        ],
    );
}

#[test]
fn a_percentage_margin_or_padding_of_an_atomic_inline_counts_as_zero_for_its_container() {
    // The container is 60px wide as if the percentages were not there; the
    // margin and the padding then resolve against it, and the atomic no
    // longer fits beside `aa`.
    check_in_each_container(
        "aa<div id=a style=\"display:inline-block;margin-left:50%\">bbbb</div>",
        &[
            ("r", (8.0, 8.0, 60.0, 20.0)),
            ("a", (38.0, 18.0, 40.0, 10.0)),
        ],
    );
    check_in_each_container(
        "aa<div id=a style=\"display:inline-block;padding-left:50%\">bbbb</div>",
        &[
            ("r", (8.0, 8.0, 60.0, 20.0)),
            ("a", (8.0, 18.0, 70.0, 10.0)),
        ],
    );
}

#[test]
fn the_percentages_of_boxes_in_a_container_with_a_width_resolve_against_it() {
    let container = "<div id=r style=\"float:left;width:200px\">{}</div>";
    for (content, expected) in [
        (
            "aa<div id=a style=\"display:inline-block;width:50%\">bbbb</div>",
            ("a", (28.0, 8.0, 100.0, 10.0)),
        ),
        (
            "aa<div id=a style=\"display:inline-block;padding-left:50%\">bbbb</div>",
            ("a", (28.0, 8.0, 140.0, 10.0)),
        ),
        (
            "aa<div id=a style=\"display:inline-block;margin-left:50%\">bbbb</div>",
            ("a", (128.0, 8.0, 40.0, 10.0)),
        ),
    ] {
        check(
            &container.replace("{}", content),
            &[("r", (8.0, 8.0, 200.0, 10.0)), expected],
        );
    }
}

#[test]
fn a_percentage_height_of_an_atomic_inline_counts_as_auto_for_its_vertical_container() {
    // The height of a vertical paragraph is its inline extent, which its
    // content decides when it shrinks to fit: `aa` and `bbbb` take 60px, and
    // the atomic then takes half of it.
    for container in ["float:left", "display:inline-block", "position:absolute"] {
        check(
            &format!(
                "<div id=r style=\"{container};writing-mode:vertical-rl;width:100px\">aa\
                 <div id=a style=\"display:inline-block;height:50%\">bbbb</div></div>"
            ),
            &[
                ("r", (8.0, 8.0, 100.0, 60.0)),
                ("a", (98.0, 28.0, 10.0, 30.0)),
            ],
        );
    }
    check(
        "<div id=r style=\"float:left;writing-mode:vertical-lr;width:100px\">aa\
         <div id=a style=\"display:inline-block;height:50%\">bbbb</div></div>",
        &[
            ("r", (8.0, 8.0, 100.0, 60.0)),
            ("a", (8.0, 28.0, 10.0, 30.0)),
        ],
    );
}

#[test]
fn a_replaced_element_with_a_percentage_limit_keeps_its_natural_width_in_a_float() {
    // The common responsive-image rule: the limit is not a width the float
    // gives up, so the float takes the 100px of the svg and the 20px of `aa`.
    let svg = |style: &str| {
        format!(
            "<div id=r style=\"float:left\">aa<svg id=a xmlns=\"http://www.w3.org/2000/svg\" \
             width=\"100\" height=\"50\" style=\"{style}\"><rect width=\"100\" height=\"50\"/></svg></div>"
        )
    };
    check(
        &svg("max-width:100%"),
        &[
            ("r", (8.0, 8.0, 120.0, 52.0)),
            ("a", (28.0, 8.0, 100.0, 50.0)),
        ],
    );
    check(
        &svg("max-width:50%"),
        &[
            ("r", (8.0, 8.0, 120.0, 52.0)),
            ("a", (28.0, 8.0, 60.0, 50.0)),
        ],
    );
}

#[test]
fn a_percentage_margin_or_padding_of_an_atomic_inline_counts_as_zero_for_a_vertical_container() {
    for (style, atomic) in [
        ("margin-top:50%", (88.0, 38.0, 10.0, 40.0)),
        ("padding-top:50%", (88.0, 8.0, 10.0, 70.0)),
    ] {
        check(
            &format!(
                "<div id=r style=\"float:left;writing-mode:vertical-rl;width:100px\">aa\
                 <div id=a style=\"display:inline-block;{style}\">bbbb</div></div>"
            ),
            &[("r", (8.0, 8.0, 100.0, 60.0)), ("a", atomic)],
        );
    }
}
