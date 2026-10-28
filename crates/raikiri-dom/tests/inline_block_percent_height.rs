//! A block inside an inline element: its percentage height and the maximum or
//! minimum height it states in percentages resolve against the height of the
//! block container around the inline (CSS 2.1 §10.1), not against the `auto`
//! height of the inline.
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
            Some((id.to_owned(), [rect.x, rect.y, rect.width, rect.height]))
        })
        .collect()
}

/// Layout units are 1/64px; allow a little more for the 32-bit sums.
const TOLERANCE: f32 = 0.04;

/// A label, the markup of a case and the `(id, left, top, width, height)` it
/// expects.
type Case<'a> = (&'a str, &'a str, &'a [(&'a str, f32, f32, f32, f32)]);

/// Lays out each case and compares the elements it names with the expected
/// boxes.
fn check_boxes(cases: &[Case<'_>]) {
    for &(label, body, expected) in cases {
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

/// The container of the cases below: 100px by 100px, so a percentage is a
/// pixel count.
fn container(inner: &str, extra: &str) -> String {
    format!("<div id=o style='width:100px;height:100px;{extra}'>{inner}</div>")
}

/// A `height: 100%` block, with a `max-height` or `min-height` that is a
/// percentage, below an inline element that holds no text.
#[test]
fn a_block_in_an_inline_resolves_its_percentage_heights_against_the_container() {
    let cases = [
        (
            "height",
            container("<span><div id=b style='height:100%'></div></span>", ""),
            vec![("b", 8.0, 8.0, 100.0, 100.0)],
        ),
        (
            "max-height",
            container(
                "<span><div id=b style='height:1000px;max-height:100%'></div></span>",
                "",
            ),
            vec![("b", 8.0, 8.0, 100.0, 100.0)],
        ),
        (
            "min-height",
            container(
                "<span><div id=b style='height:10px;min-height:50%'></div></span>",
                "",
            ),
            vec![("b", 8.0, 8.0, 100.0, 50.0)],
        ),
        (
            "two blocks",
            container(
                "<span><div id=b1 style='height:30%'></div><div id=b2 style='height:50%'></div></span>",
                "",
            ),
            vec![
                ("b1", 8.0, 8.0, 100.0, 30.0),
                ("b2", 8.0, 38.0, 100.0, 50.0),
            ],
        ),
        (
            "a relatively positioned inline",
            container(
                "<span style='position:relative'><div id=b style='height:100%'></div></span>",
                "",
            ),
            vec![("b", 8.0, 8.0, 100.0, 100.0)],
        ),
        (
            "padding of the container",
            container(
                "<span><div id=b style='height:100%'></div></span>",
                "padding:10px",
            ),
            vec![("b", 18.0, 18.0, 80.0, 80.0)],
        ),
    ];
    check(&cases);
}

/// The percentage base passes through inlines inside inlines, and stays that of
/// the container when the container is a flex item or an inline block.
#[test]
fn the_percentage_base_passes_through_nested_inlines_and_other_containers() {
    let cases = [
        (
            "nested inlines",
            container(
                "<span><span><div id=b style='height:100%'></div></span></span>",
                "",
            ),
            vec![("b", 8.0, 8.0, 100.0, 100.0)],
        ),
        (
            "an inline block",
            "<div id=o style='display:inline-block;width:100px;height:100px'>\
             <span><div id=b style='height:100%'></div></span></div>"
                .to_owned(),
            vec![("b", 8.0, 8.0, 100.0, 100.0)],
        ),
        (
            "a flex item",
            "<div style='display:flex;width:100px;height:100px'><div style='flex:1'>\
             <span><div id=b style='height:100%'></div></span></div></div>"
                .to_owned(),
            vec![("b", 8.0, 8.0, 100.0, 100.0)],
        ),
        (
            "an empty inline before the block",
            container("<span></span><div id=b style='height:100%'></div>", ""),
            vec![("b", 8.0, 8.0, 100.0, 100.0)],
        ),
        (
            "text before the block in the inline",
            container("<span>x<div id=b style='height:100%'></div></span>", ""),
            vec![("b", 8.0, 18.0, 100.0, 100.0)],
        ),
    ];
    check(&cases);
}

/// Where the container has no definite height, a percentage height behaves as
/// `auto` and a percentage maximum does not apply, as without the inline.
#[test]
fn a_container_of_auto_height_leaves_the_percentages_of_the_block_unresolved() {
    let cases = [
        (
            "height",
            "<div id=o style='width:100px'><span>\
             <div id=b style='height:100%;min-height:20px'></div></span></div>"
                .to_owned(),
            vec![("o", 8.0, 8.0, 100.0, 20.0), ("b", 8.0, 8.0, 100.0, 20.0)],
        ),
        (
            "max-height",
            "<div id=o style='width:100px'><span>\
             <div id=b style='height:60px;max-height:50%'></div></span></div>"
                .to_owned(),
            vec![("o", 8.0, 8.0, 100.0, 60.0), ("b", 8.0, 8.0, 100.0, 60.0)],
        ),
    ];
    check(&cases);
}

/// A float, an absolutely positioned box and an item of a flex or grid
/// container are blockified: they are the containing block of the block
/// inside them, so its percentage height follows their own height and not
/// the one of the container.
#[test]
fn a_blockified_inline_is_the_percentage_base_of_the_block_inside_it() {
    let cases = [
        (
            "flex item",
            "<div style='display:flex;width:100px;height:100px'><span style='height:50px'>\
             <div id=b style='height:100%'></div></span></div>"
                .to_owned(),
            vec![("b", 8.0, 8.0, 0.0, 50.0)],
        ),
        (
            "flex item with a width",
            "<div style='display:flex;width:100px;height:100px'>\
             <span style='height:50px;width:40px'><div id=b style='height:100%'></div></span></div>"
                .to_owned(),
            vec![("b", 8.0, 8.0, 40.0, 50.0)],
        ),
        (
            "flex item of auto height",
            "<div style='display:flex;align-items:flex-start;width:100px;height:100px'>\
             <span style='width:40px'><div id=b style='height:100%;min-height:20px'></div></span></div>"
                .to_owned(),
            vec![("b", 8.0, 8.0, 40.0, 20.0)],
        ),
        (
            "grid item",
            "<div style='display:grid;width:100px;height:100px'>\
             <span style='height:50px;width:40px;align-self:start'>\
             <div id=b style='height:100%'></div></span></div>"
                .to_owned(),
            vec![("b", 8.0, 8.0, 40.0, 50.0)],
        ),
        (
            "absolutely positioned",
            container(
                "<span style='position:absolute;height:50px;width:40px'>\
                 <div id=b style='height:100%'></div></span>",
                "position:relative",
            ),
            vec![("b", 8.0, 8.0, 40.0, 50.0)],
        ),
        (
            "float",
            container(
                "<span style='float:left;height:50px;width:40px'>\
                 <div id=b style='height:100%'></div></span>",
                "",
            ),
            vec![("b", 8.0, 8.0, 40.0, 50.0)],
        ),
        (
            "float of auto height",
            container(
                "<span style='float:left;width:40px'>\
                 <div id=b style='height:100%;min-height:20px'></div></span>",
                "",
            ),
            vec![("b", 8.0, 8.0, 40.0, 20.0)],
        ),
        (
            "relatively positioned inline",
            container(
                "<span style='position:relative;height:50px'>\
                 <div id=b style='height:100%'></div></span>",
                "",
            ),
            vec![("b", 8.0, 8.0, 100.0, 100.0)],
        ),
    ];
    check(&cases);
}

/// An absolutely positioned block takes its percentage height from the
/// containing block it names, with the inline between it and that block.
#[test]
fn an_absolutely_positioned_block_in_an_inline_keeps_its_containing_block() {
    check(&[(
        "absolute",
        container(
            "<span><div id=b style='position:absolute;height:50%;width:10px'></div></span>",
            "position:relative",
        ),
        vec![("b", 8.0, 8.0, 10.0, 50.0)],
    )]);
}

/// Lays out each case and compares the elements it names with the boxes
/// Chromium gives.
fn check(cases: &[(&str, String, Vec<(&str, f32, f32, f32, f32)>)]) {
    for (label, body, expected) in cases {
        check_boxes(&[(*label, body.as_str(), expected.as_slice())]);
    }
}
