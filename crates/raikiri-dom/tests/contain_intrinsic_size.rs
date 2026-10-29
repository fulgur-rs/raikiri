//! `contain-intrinsic-width`, `contain-intrinsic-height` and the
//! `contain-intrinsic-size` shorthand (CSS Box Sizing 4 §5): the size a box
//! with size containment takes in place of the size of its contents.
//!
//! The expected boxes are the ones Chromium computes for the same markup in an
//! 800px by 600px viewport, with the body margin of 8px and `10px Ahem` text:
//! each is the element's left, top, width and height.

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

const AHEM: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/data/text-autospace/Ahem.ttf"
));

/// The left, top, width and height of the elements of `body` that have an
/// `id`, on the first page.
fn boxes(body: &str) -> Vec<(String, [f32; 4])> {
    let html = format!(
        "<!DOCTYPE html><style>@page {{ size: 800px 600px; margin: 0 }} \
         body {{ margin: 8px; font: 10px/10px Ahem }}</style>{body}"
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
            Some((id.to_owned(), [rect.x, rect.y, rect.width, rect.height]))
        })
        .collect()
}

/// Layout units are 1/64px; allow a little more for the 32-bit sums.
const TOLERANCE: f32 = 0.04;

/// The `(id, left, top, width, height)` a case expects of one element.
type Expected<'a> = (&'a str, f32, f32, f32, f32);

/// Lays out the markup of each case and compares the elements it names with
/// the expected boxes.
fn check(cases: &[(&str, String, Vec<Expected<'_>>)]) {
    for (label, body, expected) in cases {
        let actual = boxes(body);
        for &(id, left, top, width, height) in expected {
            let (_, rect) = actual
                .iter()
                .find(|(name, _)| name == id)
                .unwrap_or_else(|| panic!("{label}: no fragment for #{id}"));
            let want = [left, top, width, height];
            assert!(
                rect.iter()
                    .zip(want)
                    .all(|(got, want)| (got - want).abs() <= TOLERANCE),
                "{label}: #{id} is {rect:?}, expected {want:?} (left, top, width, height)"
            );
        }
    }
}

/// A box with size containment and a `contain-intrinsic-size` of 100px by
/// 50px, plus `extra` style. It holds a line of text, which size containment
/// ignores.
fn contained(extra: &str) -> String {
    format!("<div id=b style='contain:size;contain-intrinsic-size:100px 50px;{extra}'>x y z</div>")
}

/// The box of a case that lays out as one 100px by 50px box at the corner of
/// the page.
fn corner(label: &str, markup: String) -> (&str, String, Vec<Expected<'_>>) {
    (label, markup, vec![("b", 8.0, 8.0, 100.0, 50.0)])
}

/// Where the width of the box comes from its content, the length stands in
/// for it: a keyword width, and the shrink-to-fit width of a box that is
/// inline-level, floated, absolutely positioned or an item of a flex or grid
/// container.
#[test]
fn the_length_takes_the_place_of_the_content_size_where_the_width_follows_it() {
    check(&[
        corner("max-content", contained("width:max-content")),
        corner("fit-content", contained("width:fit-content")),
        corner("min-content", contained("width:min-content")),
        corner("inline-block", contained("display:inline-block")),
        corner("float", contained("float:left")),
        corner("absolutely positioned", contained("position:absolute")),
        corner(
            "flex item",
            format!(
                "<div style='display:flex;align-items:flex-start'>{}</div>",
                contained("")
            ),
        ),
        corner(
            "grid item",
            format!(
                "<div style='display:grid;justify-items:start;align-items:start'>{}</div>",
                contained("")
            ),
        ),
        corner("flex container", contained("display:flex;width:max-content")),
        corner("grid container", contained("display:grid;width:max-content")),
        corner("inline flex container", contained("display:inline-flex")),
        corner(
            "empty box",
            "<div id=b style='contain:size;contain-intrinsic-size:100px 50px;display:inline-block'></div>"
                .to_owned(),
        ),
    ]);
}

/// A width that does not follow the content is not the length: a block fills
/// its container, and an authored width stays. The height is `auto` in both
/// and takes the length.
#[test]
fn a_width_that_does_not_follow_the_content_stays() {
    check(&[
        ("block", contained(""), vec![("b", 8.0, 8.0, 784.0, 50.0)]),
        (
            "authored width",
            contained("width:30px"),
            vec![("b", 8.0, 8.0, 30.0, 50.0)],
        ),
        (
            "authored height",
            contained("width:max-content;height:20px"),
            vec![("b", 8.0, 8.0, 100.0, 20.0)],
        ),
        (
            "the box after a float",
            format!(
                "{}<div id=c style='clear:both'>next</div>",
                contained("float:left")
            ),
            vec![("b", 8.0, 8.0, 100.0, 50.0), ("c", 8.0, 58.0, 784.0, 10.0)],
        ),
    ]);
}

/// The length is the size of the content box; the padding and the border come
/// on top whatever the `box-sizing` is, and the minimum and maximum sizes
/// still clamp the result.
#[test]
fn the_length_is_the_content_size_inside_the_padding_the_border_and_the_limits() {
    check(&[
        (
            "padding and border",
            contained("width:max-content;padding:5px;border:3px solid"),
            vec![("b", 8.0, 8.0, 116.0, 66.0)],
        ),
        (
            "border-box",
            contained("width:max-content;padding:5px;border:3px solid;box-sizing:border-box"),
            vec![("b", 8.0, 8.0, 116.0, 66.0)],
        ),
        (
            "authored border-box width",
            contained("width:90px;padding:5px;border:3px solid;box-sizing:border-box"),
            vec![("b", 8.0, 8.0, 90.0, 66.0)],
        ),
        (
            "min-height",
            contained("width:max-content;min-height:80px"),
            vec![("b", 8.0, 8.0, 100.0, 80.0)],
        ),
        (
            "max-height",
            contained("width:max-content;max-height:30px"),
            vec![("b", 8.0, 8.0, 100.0, 30.0)],
        ),
    ]);
}

/// Each longhand gives its own axis; an axis without a length (`none`, the
/// initial value) has no size. `auto` falls back to the length when there is
/// no size remembered from skipped contents, which a page never has.
#[test]
fn the_longhands_give_one_axis_each_and_none_gives_no_size() {
    let alone = |style: &str| {
        format!("<div id=b style='contain:size;width:max-content;{style}'>x y z</div>")
    };
    check(&[
        (
            "width only",
            alone("contain-intrinsic-width:100px"),
            vec![("b", 8.0, 8.0, 100.0, 0.0)],
        ),
        (
            "height only",
            alone("contain-intrinsic-height:50px"),
            vec![("b", 8.0, 8.0, 0.0, 50.0)],
        ),
        (
            "auto and a length",
            alone("contain-intrinsic-size:auto 100px auto 50px"),
            vec![("b", 8.0, 8.0, 100.0, 50.0)],
        ),
        (
            "auto none",
            alone("contain-intrinsic-size:auto none"),
            vec![("b", 8.0, 8.0, 0.0, 0.0)],
        ),
        (
            "none",
            alone("contain-intrinsic-size:none"),
            vec![("b", 8.0, 8.0, 0.0, 0.0)],
        ),
        (
            "one value for both axes",
            contained("width:max-content;contain-intrinsic-size:60px"),
            vec![("b", 8.0, 8.0, 60.0, 60.0)],
        ),
        (
            "font-relative lengths",
            contained("width:max-content;contain-intrinsic-size:3em 2em"),
            vec![("b", 8.0, 8.0, 30.0, 20.0)],
        ),
        (
            "a longhand after the shorthand",
            contained("width:max-content;contain-intrinsic-height:20px"),
            vec![("b", 8.0, 8.0, 100.0, 20.0)],
        ),
        (
            "both longhands after the shorthand",
            contained("width:max-content;contain-intrinsic-width:1em;contain-intrinsic-height:2em"),
            vec![("b", 8.0, 8.0, 10.0, 20.0)],
        ),
    ]);
}

/// A grid container keeps the size its explicit tracks give an empty grid in
/// an axis that states no length; the axis that states one takes it.
#[test]
fn a_grid_keeps_its_tracks_in_the_axis_that_states_no_length() {
    let grid = |style: &str| {
        format!(
            "<div id=b style='display:grid;grid-template-columns:50px 50px;\
             grid-template-rows:30px 30px;contain:size;{style}'>x y z</div>"
        )
    };
    check(&[
        (
            "tracks alone",
            grid("width:max-content"),
            vec![("b", 8.0, 8.0, 100.0, 60.0)],
        ),
        (
            "none for both",
            grid("width:max-content;contain-intrinsic-size:none"),
            vec![("b", 8.0, 8.0, 100.0, 60.0)],
        ),
        (
            "a width",
            grid("width:max-content;contain-intrinsic-width:80px"),
            vec![("b", 8.0, 8.0, 80.0, 60.0)],
        ),
        (
            "a height",
            grid("width:max-content;contain-intrinsic-height:40px"),
            vec![("b", 8.0, 8.0, 100.0, 40.0)],
        ),
        (
            "both",
            grid("width:max-content;contain-intrinsic-size:80px 40px"),
            vec![("b", 8.0, 8.0, 80.0, 40.0)],
        ),
        (
            "a block grid, width",
            grid("contain-intrinsic-width:80px"),
            vec![("b", 8.0, 8.0, 784.0, 60.0)],
        ),
        (
            "a block grid, height",
            grid("contain-intrinsic-height:40px"),
            vec![("b", 8.0, 8.0, 784.0, 40.0)],
        ),
        (
            "a float, width",
            grid("float:left;contain-intrinsic-width:80px"),
            vec![("b", 8.0, 8.0, 80.0, 60.0)],
        ),
        (
            "padding and border, width",
            grid("width:max-content;padding:5px;border:3px solid;contain-intrinsic-width:80px"),
            vec![("b", 8.0, 8.0, 96.0, 76.0)],
        ),
        (
            "padding and border, height",
            grid("width:max-content;padding:5px;border:3px solid;contain-intrinsic-height:40px"),
            vec![("b", 8.0, 8.0, 116.0, 56.0)],
        ),
        (
            "min-height",
            grid("width:max-content;min-height:100px;contain-intrinsic-width:80px"),
            vec![("b", 8.0, 8.0, 80.0, 100.0)],
        ),
        (
            "inline-size containment",
            grid("width:max-content;contain:inline-size;contain-intrinsic-width:80px"),
            vec![("b", 8.0, 8.0, 80.0, 60.0)],
        ),
        (
            "an inline grid, width",
            "<div id=b style='display:inline-grid;contain:size;contain-intrinsic-width:100px;\
             grid-template-rows:15px'>x y z</div>"
                .to_owned(),
            vec![("b", 8.0, 8.0, 100.0, 15.0)],
        ),
        (
            "an inline grid, height",
            "<div id=b style='display:inline-grid;contain:size;contain-intrinsic-height:40px;\
             grid-template-columns:70px'>x y z</div>"
                .to_owned(),
            vec![("b", 8.0, 8.0, 70.0, 40.0)],
        ),
        (
            "a flex item, width",
            format!(
                "<div style='display:flex;align-items:flex-start'>{}</div>",
                grid("contain-intrinsic-width:80px")
            ),
            vec![("b", 8.0, 8.0, 80.0, 60.0)],
        ),
    ]);
}

/// The lengths are computed like any other: `calc()` and `var()` give them,
/// `inherit` takes the value of the parent, and the property does not inherit
/// by itself.
#[test]
fn functions_and_css_wide_keywords_give_the_lengths() {
    let alone = |style: &str| {
        format!("<div id=b style='contain:size;width:max-content;{style}'>x y z</div>")
    };
    check(&[
        (
            "calc",
            alone("contain-intrinsic-size:calc(40px + 60px) calc(2 * 25px)"),
            vec![("b", 8.0, 8.0, 100.0, 50.0)],
        ),
        (
            "var",
            alone(
                "--w:100px;contain-intrinsic-width:var(--w);contain-intrinsic-height:var(--h, 50px)",
            ),
            vec![("b", 8.0, 8.0, 100.0, 50.0)],
        ),
        (
            "calc of mixed units, longhands",
            alone(
                "font-size:10px;contain-intrinsic-width:calc(2em + 10px);\
                 contain-intrinsic-height:calc(1em + 5px)",
            ),
            vec![("b", 8.0, 8.0, 30.0, 15.0)],
        ),
        (
            "calc of mixed units, shorthand",
            alone("font-size:10px;contain-intrinsic-size:calc(2em + 10px) calc(1em + 5px)"),
            vec![("b", 8.0, 8.0, 30.0, 15.0)],
        ),
        (
            "calc of mixed units after auto",
            alone("font-size:10px;contain-intrinsic-size:auto calc(2em + 10px) none"),
            vec![("b", 8.0, 8.0, 30.0, 0.0)],
        ),
        (
            "calc of rem and viewport units",
            alone("contain-intrinsic-size:calc(1rem + 4px) calc(10vw + 20px)"),
            vec![("b", 8.0, 8.0, 20.0, 100.0)],
        ),
        (
            "a negative calc is clamped",
            alone("font-size:10px;contain-intrinsic-size:calc(10px - 3em) 20px"),
            vec![("b", 8.0, 8.0, 0.0, 20.0)],
        ),
        (
            "calc of mixed units through var",
            alone("font-size:10px;--w:2em;contain-intrinsic-size:calc(var(--w) + 10px) 50px"),
            vec![("b", 8.0, 8.0, 30.0, 50.0)],
        ),
        (
            "calc with a percentage",
            alone("contain-intrinsic-size:100px 50px;contain-intrinsic-width:calc(10px + 50%)"),
            vec![("b", 8.0, 8.0, 100.0, 50.0)],
        ),
        (
            "inherit",
            format!(
                "<div style='contain-intrinsic-size:30px 20px'>{}</div>",
                alone("contain-intrinsic-size:inherit")
            ),
            vec![("b", 8.0, 8.0, 30.0, 20.0)],
        ),
        (
            "initial",
            alone("contain-intrinsic-size:100px 50px;contain-intrinsic-width:initial"),
            vec![("b", 8.0, 8.0, 0.0, 50.0)],
        ),
        (
            "not inherited",
            format!(
                "<div style='contain-intrinsic-size:30px 20px'>{}</div>",
                alone("")
            ),
            vec![("b", 8.0, 8.0, 0.0, 0.0)],
        ),
        (
            "rem",
            alone("contain-intrinsic-size:2rem 1rem"),
            vec![("b", 8.0, 8.0, 32.0, 16.0)],
        ),
        (
            "auto with a length",
            alone("contain-intrinsic-size:auto 80px auto 40px"),
            vec![("b", 8.0, 8.0, 80.0, 40.0)],
        ),
    ]);
}

/// A percentage and a negative length are not valid, so the declaration is
/// dropped and the shorthand before it stands.
#[test]
fn an_invalid_declaration_is_dropped() {
    check(&[
        corner(
            "percentage",
            contained("width:max-content;contain-intrinsic-size:50% 50%"),
        ),
        corner(
            "negative length",
            contained("width:max-content;contain-intrinsic-size:-10px"),
        ),
    ]);
}

/// The lengths act only in the axes that size containment covers: without
/// `contain`, the content sizes the box, and `contain: inline-size` leaves the
/// block size to the content.
#[test]
fn the_length_acts_only_in_the_axes_that_are_contained() {
    check(&[
        (
            "no containment",
            "<div id=b style='contain-intrinsic-size:100px 50px;width:max-content'>x y z</div>"
                .to_owned(),
            vec![("b", 8.0, 8.0, 50.0, 10.0)],
        ),
        (
            "inline-size containment",
            "<div id=b style='contain:inline-size;contain-intrinsic-size:100px 50px;width:max-content'>x y z</div>"
                .to_owned(),
            vec![("b", 8.0, 8.0, 100.0, 10.0)],
        ),
        (
            "inline-size containment, inline-block",
            "<div id=b style='contain:inline-size;contain-intrinsic-size:100px 50px;display:inline-block'>x y z</div>"
                .to_owned(),
            vec![("b", 8.0, 8.0, 100.0, 10.0)],
        ),
        (
            "table",
            contained("display:table"),
            vec![("b", 8.0, 8.0, 50.0, 10.0)],
        ),
    ]);
}

/// The lengths are physical: the width is the horizontal size whatever the
/// writing mode is.
#[test]
fn the_width_and_the_height_stay_physical_in_a_vertical_writing_mode() {
    check(&[corner(
        "vertical-rl",
        contained("width:max-content;writing-mode:vertical-rl"),
    )]);
}

/// A replaced element with size containment has no natural size either, and
/// the length gives it one.
#[test]
fn a_replaced_element_takes_the_length_as_its_natural_size() {
    check(&[corner(
        "image",
        "<img id=b style='contain:size;contain-intrinsic-size:100px 50px' \
         src='data:image/gif;base64,R0lGODlhAQABAIAAAAAAAP///yH5BAEAAAAALAAAAAABAAEAAAIBRAA7'>"
            .to_owned(),
    )]);
}
