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

/// An svg of 100px by 50px with the id `a` and `style`.
fn svg_100_by_50(style: &str) -> String {
    format!(
        "<svg id=a xmlns=\"http://www.w3.org/2000/svg\" width=\"100\" height=\"50\" \
         style=\"{style}\"><rect width=\"100\" height=\"50\"/></svg>"
    )
}

#[test]
fn a_percentage_size_of_a_replaced_element_counts_as_auto_for_its_shrink_to_fit_container() {
    // The container takes the 20px of `aa` and the 100px of the svg, and the
    // svg then resolves its percentages against the 120px.
    for (style, container, svg) in [
        ("max-width:100%", (120.0, 52.0), (28.0, 8.0, 100.0, 50.0)),
        ("width:50%", (120.0, 52.0), (28.0, 8.0, 60.0, 50.0)),
        // The whole 120px leaves no room beside `aa`.
        ("width:100%", (120.0, 62.0), (8.0, 18.0, 120.0, 50.0)),
        ("min-width:50%", (120.0, 52.0), (28.0, 8.0, 100.0, 50.0)),
        // The `max-width` of 80px limits the contribution, not the percentage.
        (
            "width:50%;max-width:80px",
            (100.0, 52.0),
            (28.0, 8.0, 50.0, 50.0),
        ),
    ] {
        check_in_each_container(
            &format!("aa{}", svg_100_by_50(style)),
            &[("r", (8.0, 8.0, container.0, container.1)), ("a", svg)],
        );
    }
}

#[test]
fn a_percentage_size_of_a_replaced_element_resolves_against_zero_for_a_min_content_container() {
    // The percentages shrink the svg to what `aa` leaves; `min-width` does not
    // count, and the 100px of the svg is what its container takes.
    for (style, container, svg) in [
        ("max-width:100%", (20.0, 62.0), (8.0, 18.0, 20.0, 50.0)),
        ("width:50%", (20.0, 62.0), (8.0, 18.0, 10.0, 50.0)),
        ("width:100%", (20.0, 62.0), (8.0, 18.0, 20.0, 50.0)),
        ("min-width:50%", (100.0, 62.0), (8.0, 18.0, 100.0, 50.0)),
        (
            "width:50%;max-width:80px",
            (20.0, 62.0),
            (8.0, 18.0, 10.0, 50.0),
        ),
    ] {
        check(
            &format!(
                "<div id=r style=\"width:min-content\">aa{}</div>",
                svg_100_by_50(style)
            ),
            &[("r", (8.0, 8.0, container.0, container.1)), ("a", svg)],
        );
    }
}

#[test]
fn a_percentage_height_of_a_replaced_element_counts_as_auto_for_the_width_of_a_min_content_container()
 {
    // A canvas of 25px by 50px. Its container measures the width only, and a
    // percentage height stays `auto` there, so the width its ratio carries is
    // the natural one; only a percentage width resolves against zero.
    for (style, container, canvas) in [
        ("height:50%", (25.0, 50.0), (8.0, 8.0, 25.0, 50.0)),
        ("height:100%", (25.0, 50.0), (8.0, 8.0, 25.0, 50.0)),
        ("max-height:50%", (25.0, 50.0), (8.0, 8.0, 25.0, 50.0)),
        ("min-height:50%", (25.0, 50.0), (8.0, 8.0, 25.0, 50.0)),
        (
            "height:calc(50% + 40px)",
            (25.0, 50.0),
            (8.0, 8.0, 25.0, 50.0),
        ),
        (
            "height:50%;min-width:min-content",
            (25.0, 50.0),
            (8.0, 8.0, 25.0, 50.0),
        ),
        // The width the box states carries its height along the natural ratio.
        (
            "height:50%;width:40px",
            (40.0, 80.0),
            (8.0, 8.0, 40.0, 80.0),
        ),
        // A ratio the box states replaces the natural one.
        (
            "height:50%;aspect-ratio:1/1",
            (25.0, 25.0),
            (8.0, 8.0, 25.0, 25.0),
        ),
        ("width:50%", (0.0, 0.0), (8.0, 8.0, 0.0, 0.0)),
    ] {
        check(
            &format!(
                "<div id=r style=\"width:min-content\"><canvas id=a width=25 height=50 \
                 style=\"display:block;{style}\"></canvas></div>"
            ),
            &[("r", (8.0, 8.0, container.0, container.1)), ("a", canvas)],
        );
    }
}

/// A canvas of 25px by 50px with the id `a` and `style`.
fn canvas_25_by_50(style: &str) -> String {
    format!("<canvas id=a width=25 height=50 style=\"{style}\"></canvas>")
}

#[test]
fn a_calc_with_a_percentage_of_a_replaced_element_contributes_nothing_to_a_min_content_container() {
    // A size with a percentage anywhere in it makes the canvas compressible, so
    // the container is as wide as the canvas contributes, whatever length the
    // percentage is added to; the canvas then resolves the whole `calc()`
    // against that width. The canvas is inline, so a line holds it.
    for (style, container, canvas) in [
        (
            "width:calc(50% + 10px)",
            (0.0, 22.0),
            (8.0, 8.0, 10.0, 20.0),
        ),
        (
            "width:calc(100% + 10px)",
            (0.0, 22.0),
            (8.0, 8.0, 10.0, 20.0),
        ),
        // A negative width is zero.
        ("width:calc(50% - 10px)", (0.0, 10.0), (8.0, 16.0, 0.0, 0.0)),
        // The maximum clamps the canvas, not its contribution.
        (
            "width:calc(50% + 10px);max-width:5px",
            (0.0, 12.0),
            (8.0, 8.0, 5.0, 10.0),
        ),
        // The minimum counts.
        (
            "width:calc(50% + 10px);min-width:20px",
            (20.0, 42.0),
            (8.0, 8.0, 20.0, 40.0),
        ),
        // The border is part of the contribution, the content is not.
        (
            "width:calc(50% + 10px);border:3px solid",
            (6.0, 34.0),
            (8.0, 8.0, 19.0, 32.0),
        ),
        (
            "width:calc(50% + 10px);height:30px",
            (0.0, 32.0),
            (8.0, 8.0, 10.0, 30.0),
        ),
        // A `calc()` in the maximum is a limit of zero for the contribution,
        // whatever it comes to for the canvas, and whether the width is
        // `auto` or not.
        (
            "max-width:calc(50% + 10px)",
            (0.0, 22.0),
            (8.0, 8.0, 10.0, 20.0),
        ),
        (
            "max-width:calc(50% + 40px)",
            (0.0, 52.0),
            (8.0, 8.0, 25.0, 50.0),
        ),
        (
            "width:40px;max-width:calc(50% + 10px)",
            (0.0, 22.0),
            (8.0, 8.0, 10.0, 20.0),
        ),
        // The length of a `calc()` in the minimum is part of it.
        (
            "min-width:calc(50% + 40px)",
            (40.0, 122.0),
            (8.0, 8.0, 60.0, 120.0),
        ),
        (
            "width:calc(50% + 10px);min-width:calc(50% + 40px)",
            (40.0, 122.0),
            (8.0, 8.0, 60.0, 120.0),
        ),
    ] {
        check(
            &format!(
                "<div id=r style=\"width:min-content\">{}</div>",
                canvas_25_by_50(style)
            ),
            &[("r", (8.0, 8.0, container.0, container.1)), ("a", canvas)],
        );
    }
}

#[test]
fn a_calc_with_a_percentage_of_a_block_replaced_element_contributes_nothing_to_a_min_content_container()
 {
    for (style, container, canvas) in [
        (
            "width:calc(50% + 10px)",
            (0.0, 20.0),
            (8.0, 8.0, 10.0, 20.0),
        ),
        (
            "width:calc(100% + 10px)",
            (0.0, 20.0),
            (8.0, 8.0, 10.0, 20.0),
        ),
        (
            "max-width:calc(50% + 10px)",
            (0.0, 20.0),
            (8.0, 8.0, 10.0, 20.0),
        ),
    ] {
        check(
            &format!(
                "<div id=r style=\"width:min-content\">{}</div>",
                canvas_25_by_50(&format!("display:block;{style}"))
            ),
            &[("r", (8.0, 8.0, container.0, container.1)), ("a", canvas)],
        );
    }
}

#[test]
fn a_calc_with_a_percentage_of_a_replaced_grid_item_contributes_nothing_to_a_min_content_container()
{
    for (style, container, canvas) in [
        (
            "width:calc(50% + 10px)",
            (0.0, 20.0),
            (8.0, 8.0, 10.0, 20.0),
        ),
        (
            "max-width:calc(50% + 10px)",
            (0.0, 20.0),
            (8.0, 8.0, 10.0, 20.0),
        ),
        ("width:50%", (0.0, 0.0), (8.0, 8.0, 0.0, 0.0)),
    ] {
        check(
            &format!(
                "<div id=r style=\"display:grid;width:min-content\">{}</div>",
                canvas_25_by_50(style)
            ),
            &[("r", (8.0, 8.0, container.0, container.1)), ("a", canvas)],
        );
    }
}

#[test]
fn a_calc_with_a_percentage_of_a_replaced_element_keeps_its_length_in_a_max_content_container() {
    // A container that shrinks to fit takes the canvas as it is without the
    // percentage, then resolves the `calc()` against the width it ends up with:
    // half of 25px and 10px.
    for container in ["float:left", "display:inline-block", "width:fit-content"] {
        check(
            &format!(
                "<div id=r style=\"{container}\">{}</div>",
                canvas_25_by_50("width:calc(50% + 10px)")
            ),
            &[("r", (8.0, 8.0, 25.0, 47.0)), ("a", (8.0, 8.0, 22.5, 45.0))],
        );
    }
}

#[test]
fn a_calc_with_a_percentage_of_an_svg_contributes_nothing_to_a_min_content_container() {
    // An svg with a `viewBox` and no size of its own: inline in a line, and
    // alone as a block.
    let svg = |style: &str| {
        format!(
            "<svg id=a xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 100 50\" \
             style=\"{style}\"><rect width=\"100\" height=\"50\"/></svg>"
        )
    };
    for (style, container, svg_box) in [
        (
            "width:calc(50% + 10px)",
            (0.0, 10.0),
            (8.0, 11.0, 10.0, 5.0),
        ),
        ("width:50%", (0.0, 10.0), (8.0, 16.0, 0.0, 0.0)),
        (
            "max-width:calc(50% + 10px)",
            (0.0, 10.0),
            (8.0, 16.0, 0.0, 0.0),
        ),
        (
            "display:block;width:calc(50% + 10px)",
            (0.0, 5.0),
            (8.0, 8.0, 10.0, 5.0),
        ),
    ] {
        check(
            &format!("<div id=r style=\"width:min-content\">{}</div>", svg(style)),
            &[("r", (8.0, 8.0, container.0, container.1)), ("a", svg_box)],
        );
    }
}

#[test]
fn a_percentage_margin_of_a_replaced_element_counts_as_zero_for_its_container() {
    // The container is as wide as the svg, and the margin then resolves
    // against that width: 10% of 100px.
    for (style, svg) in [
        ("margin-left:10%", (18.0, 8.0, 100.0, 50.0)),
        ("margin:0 10%", (18.0, 8.0, 100.0, 50.0)),
        ("margin-left:10%;width:50%", (18.0, 8.0, 50.0, 50.0)),
        ("margin-left:10%;max-width:100%", (18.0, 8.0, 100.0, 50.0)),
    ] {
        check_in_each_container(
            &svg_100_by_50(style),
            &[("r", (8.0, 8.0, 100.0, 52.0)), ("a", svg)],
        );
    }
    check(
        &format!(
            "<div id=r style=\"width:min-content\">{}</div>",
            svg_100_by_50("margin-left:10%")
        ),
        &[
            ("r", (8.0, 8.0, 100.0, 52.0)),
            ("a", (18.0, 8.0, 100.0, 50.0)),
        ],
    );
}

/// The vertical containers whose height is decided by their content.
const VERTICAL_CONTAINERS: [&str; 3] = [
    "<div id=r style=\"float:left;writing-mode:vertical-rl\">{}</div>",
    "<div id=r style=\"display:inline-block;writing-mode:vertical-rl\">{}</div>",
    "<div style=\"display:table\"><div id=r style=\"display:table-cell;\
     writing-mode:vertical-rl\">{}</div></div>",
];

#[test]
fn a_percentage_size_of_a_replaced_element_counts_as_auto_for_its_vertical_container() {
    // The height of a vertical container is its inline extent: the svg takes
    // its 50px, and the percentage then resolves against that.
    for (style, svg) in [
        ("max-height:100%", (8.0, 8.0, 100.0, 50.0)),
        ("height:50%", (8.0, 8.0, 100.0, 25.0)),
        ("height:100%", (8.0, 8.0, 100.0, 50.0)),
        ("min-height:50%", (8.0, 8.0, 100.0, 50.0)),
        ("margin-top:10%", (8.0, 13.0, 100.0, 50.0)),
    ] {
        for container in VERTICAL_CONTAINERS {
            check(
                &container.replace("{}", &svg_100_by_50(style)),
                &[("r", (8.0, 8.0, 100.0, 50.0)), ("a", svg)],
            );
        }
    }
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

/// `depth` boxes with `style` inside each other around `bbbb`; the outermost
/// has the id `a` and the innermost the id `z`.
fn nested(style: &str, depth: usize) -> String {
    (0..depth).fold(String::from("bbbb"), |inner, level| {
        let id = if level == depth - 1 {
            " id=a"
        } else if level == 0 {
            " id=z"
        } else {
            ""
        };
        format!("<div{id} style=\"{style}\">{inner}</div>")
    })
}

#[test]
fn nested_percentage_width_boxes_each_take_half_of_the_one_around_them() {
    // Every box counts as `auto` for the container, which is 60px wide; the
    // boxes then halve the width one after the other.
    check(
        &format!(
            "<div id=r style=\"float:left\">aa{}</div>",
            nested("display:inline-block;width:50%", 3)
        ),
        &[
            ("r", (8.0, 8.0, 60.0, 10.0)),
            ("a", (28.0, 8.0, 30.0, 10.0)),
            ("z", (28.0, 8.0, 7.5, 10.0)),
        ],
    );
    check(
        &format!(
            "<div id=r style=\"float:left\">aa{}</div>",
            nested("float:left;width:50%", 3)
        ),
        &[
            ("r", (8.0, 8.0, 60.0, 10.0)),
            ("a", (8.0, 8.0, 30.0, 10.0)),
            ("z", (8.0, 8.0, 7.5, 10.0)),
        ],
    );
}

#[test]
fn a_chain_of_nested_percentage_width_boxes_is_laid_out_in_a_time_that_grows_slowly() {
    // Measuring a box must not make the boxes around it forget what they have
    // measured: the time then multiplies for every box of the chain. The chain
    // takes milliseconds; with the multiplication it takes about twenty seconds.
    for style in ["display:inline-block;width:50%", "float:left;width:50%"] {
        let start = std::time::Instant::now();
        check(
            &format!(
                "<div id=r style=\"float:left\">aa{}</div>",
                nested(style, 8)
            ),
            &[("r", (8.0, 8.0, 60.0, 10.0))],
        );
        assert!(
            start.elapsed() < std::time::Duration::from_secs(5),
            "{style}: the chain took {:?}",
            start.elapsed()
        );
    }
}
