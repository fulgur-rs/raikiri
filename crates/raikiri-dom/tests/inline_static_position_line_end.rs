//! The static position of an inline-level absolutely positioned box whose
//! anchor follows the spaces at the end of a line, in paragraphs and text of
//! either direction.
//!
//! Collapsible spaces at the end of a line are removed (CSS Text 3, 4.1.3),
//! so the box sits where the content of the line ends, which is its left end
//! in a right-to-left line. A box whose anchor the line breaker put on the
//! next line, right after the break, stays at the end of the line before.
//! There it takes the direction of the text on both sides of it when they
//! agree (UAX #9, rule N1): text that reads against the line's direction
//! ends at the far side of its run, and the box with it.
//!
//! The expected boxes are the ones Chromium computes for the same markup,
//! inside 20px of padding, with no body margin and `10px/10px Ahem` text:
//! each is the box's left, top, width and height. U+202E and U+202C override
//! the direction of Latin text.

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

const AHEM: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/data/text-autospace/Ahem.ttf"
));

/// The left, top, width and height of the element with id `q` in `body`, on
/// the first page.
fn positioned_box(body: &str) -> [f32; 4] {
    let html = format!(
        "<!DOCTYPE html><style>@page {{ size: 800px 600px; margin: 0 }} \
         html, body {{ margin: 0 }} body {{ font: 10px/10px Ahem }}</style>{body}"
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
        .find(|fragment| dom.attr(fragment.node(), "id") == Some("q"))
        .map(|fragment| {
            let rect = fragment.rect();
            [rect.x, rect.y, rect.width, rect.height]
        })
        .expect("the positioned box")
}

/// Layout units are 1/64px; allow a little more for the 32-bit sums.
const TOLERANCE: f32 = 0.04;

/// A label, the markup of a case and the box it expects.
const CASES: &[(&str, &str, [f32; 4])] = &[
    (
        "rtl left after a space at the end of the paragraph",
        "<div style='padding-left:20px'><div style='direction:rtl;text-align:left;width:100px'>XX <span id=q style='position:absolute'>Q</span></div></div>",
        [10.0, 0.0, 10.0, 10.0],
    ),
    (
        "rtl center after a space at the end of the paragraph",
        "<div style='padding-left:20px'><div style='direction:rtl;text-align:center;width:100px'>XX <span id=q style='position:absolute'>Q</span></div></div>",
        [50.0, 0.0, 10.0, 10.0],
    ),
    (
        "rtl right after two spaces at the end of the paragraph",
        "<div style='padding-left:20px'><div style='direction:rtl;text-align:right;width:100px'>XX  <span id=q style='position:absolute'>Q</span></div></div>",
        [90.0, 0.0, 10.0, 10.0],
    ),
    (
        "rtl left after the space where the line wraps",
        "<div style='padding-left:20px'><div style='direction:rtl;text-align:left;width:100px'>XXXX XXXX <span id=q style='position:absolute'>Q</span>YYYY</div></div>",
        [110.0, 0.0, 10.0, 10.0],
    ),
    (
        "rtl center after the space where the line wraps",
        "<div style='padding-left:20px'><div style='direction:rtl;text-align:center;width:100px'>XXXX XXXX <span id=q style='position:absolute'>Q</span>YYYY</div></div>",
        [115.0, 0.0, 10.0, 10.0],
    ),
    (
        "rtl right after the space where a shorter line wraps",
        "<div style='padding-left:20px'><div style='direction:rtl;text-align:right;width:100px'>XXXX XXX <span id=q style='position:absolute'>Q</span>YYYY</div></div>",
        [120.0, 0.0, 10.0, 10.0],
    ),
    (
        "ltr left after a space at the end of the paragraph",
        "<div style='padding-left:20px'><div style='direction:ltr;text-align:left;width:100px'>XX <span id=q style='position:absolute'>Q</span></div></div>",
        [40.0, 0.0, 10.0, 10.0],
    ),
    (
        "ltr right after the space where the line wraps",
        "<div style='padding-left:20px'><div style='direction:ltr;text-align:right;width:100px'>XXXX XXXX <span id=q style='position:absolute'>Q</span>YYYY</div></div>",
        [120.0, 0.0, 10.0, 10.0],
    ),
    (
        "rtl left after preserved spaces at the end",
        "<div style='padding-left:20px'><div style='direction:rtl;text-align:left;width:100px;white-space:pre-wrap'>XX  <span id=q style='position:absolute'>Q</span></div></div>",
        [10.0, 0.0, 10.0, 10.0],
    ),
    (
        "rtl center between words",
        "<div style='padding-left:20px'><div style='direction:rtl;text-align:center;width:100px'>XX <span id=q style='position:absolute'>Q</span>YY</div></div>",
        [75.0, 0.0, 10.0, 10.0],
    ),
    (
        "rtl left after an overridden word and a space at the end",
        "<div style='padding-left:20px'><div style='direction:rtl;text-align:left;width:100px'>XX \u{202E}YY\u{202C} <span id=q style='position:absolute'>Q</span></div></div>",
        [10.0, 0.0, 10.0, 10.0],
    ),
    (
        "ltr right after an overridden word and a space at the end",
        "<div style='padding-left:20px'><div style='direction:ltr;text-align:right;width:100px'>XX \u{202E}YY\u{202C} <span id=q style='position:absolute'>Q</span></div></div>",
        [120.0, 0.0, 10.0, 10.0],
    ),
    (
        "rtl left after an overridden word where the line wraps",
        "<div style='padding-left:20px'><div style='direction:rtl;text-align:left;width:100px'>XXXX \u{202E}XXXX\u{202C} <span id=q style='position:absolute'>Q</span>YYYY</div></div>",
        [10.0, 0.0, 10.0, 10.0],
    ),
    (
        "ltr left after an overridden word where the line wraps",
        "<div style='padding-left:20px'><div style='direction:ltr;text-align:left;width:100px'>XXXX \u{202E}XXXX\u{202C} <span id=q style='position:absolute'>Q</span>YYYY</div></div>",
        [110.0, 0.0, 10.0, 10.0],
    ),
    (
        "rtl right before an overridden word on the next line",
        "<div style='padding-left:20px'><div style='direction:rtl;text-align:right;width:100px'>XXXX XXXX <span id=q style='position:absolute'>Q</span>\u{202E}YYYY\u{202C}</div></div>",
        [20.0, 0.0, 10.0, 10.0],
    ),
    (
        "rtl left between overridden words across a wrap",
        "<div style='padding-left:20px'><div style='direction:rtl;text-align:left;width:100px'>\u{202E}XXXX XXXX\u{202C} <span id=q style='position:absolute'>Q</span>\u{202E}YYYY\u{202C}</div></div>",
        [10.0, 0.0, 10.0, 10.0],
    ),
    (
        "ltr left between overridden words across a wrap",
        "<div style='padding-left:20px'><div style='direction:ltr;text-align:left;width:100px'>\u{202E}XXXX XXXX\u{202C} <span id=q style='position:absolute'>Q</span>\u{202E}YYYY\u{202C}</div></div>",
        [10.0, 0.0, 10.0, 10.0],
    ),
    (
        "ltr left between an overridden word and a plain one",
        "<div style='padding-left:20px'><div style='direction:ltr;text-align:left;width:100px'>XX \u{202E}YY\u{202C} <span id=q style='position:absolute'>Q</span>ZZ</div></div>",
        [80.0, 0.0, 10.0, 10.0],
    ),
    (
        "rtl left with another box after it where the line wraps",
        "<div style='padding-left:20px'><div style='direction:rtl;text-align:left;width:100px'>XXXX XXXX <span id=q style='position:absolute'>Q</span><span style='position:absolute'>S</span>YYYY</div></div>",
        [110.0, 0.0, 10.0, 10.0],
    ),
    (
        "ltr right with another box after it where the line wraps",
        "<div style='padding-left:20px'><div style='direction:ltr;text-align:right;width:100px'>XXXX XXXX <span id=q style='position:absolute'>Q</span><span style='position:absolute'>S</span>YYYY</div></div>",
        [120.0, 0.0, 10.0, 10.0],
    ),
];

#[test]
fn a_box_after_the_spaces_at_a_line_end_takes_the_chromium_static_position() {
    let mismatches: Vec<String> = CASES
        .iter()
        .filter_map(|&(label, body, expected)| {
            let actual = positioned_box(body);
            let differs = actual
                .iter()
                .zip(expected)
                .any(|(a, e)| (a - e).abs() > TOLERANCE);
            differs.then(|| format!("{label}: expected {expected:?}, got {actual:?}"))
        })
        .collect();
    assert!(mismatches.is_empty(), "{}", mismatches.join("\n"));
}
