//! The static position of an inline-level absolutely positioned box that
//! sits on a line of its own: before or between block-level siblings, where
//! no text shares its line.
//!
//! Chromium places the box on a line that holds nothing else, set in by the
//! floats beside it. That line is aligned by `text-align` alone, and
//! `text-indent` moves its aligned point unless the indent is `hanging`.
//! When the box follows a forced line break or sits in an inline box, the line
//! is an ordinary last line instead: `text-align-last` aligns it, and the
//! indent of a first line narrows it. Block flow places such a box on the
//! same empty line as a paragraph does.
//!
//! The expected boxes are the ones Chromium computes for the same markup, with
//! no body margin and `10px/10px Ahem` text: each is the box's left, top,
//! width and height.

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
        "center after a block",
        r#"<div id=c style="text-align:center;direction:ltr;width:100px">X<p style="margin:0;width:20px">A</p><span id=q style="position:absolute">Q</span><p style="margin:0;width:20px">B</p>T</div>"#,
        [50.0, 20.0, 10.0, 10.0],
    ),
    (
        "right before an empty block at the end",
        r#"<div id=c style="text-align:right;direction:ltr;width:100px">X<p style="margin:0;width:20px">A</p><span id=q style="position:absolute">Q</span><div></div></div>"#,
        [100.0, 20.0, 10.0, 10.0],
    ),
    (
        "end in rtl",
        r#"<div id=c style="text-align:end;direction:rtl;width:100px">X<p style="margin:0;width:20px">A</p><span id=q style="position:absolute">Q</span><p style="margin:0;width:20px">B</p>T</div>"#,
        [-10.0, 20.0, 10.0, 10.0],
    ),
    (
        "center in rtl beside a left float",
        r#"<div id=c style="text-align:center;direction:rtl;width:100px"><div style="float:left;width:40px;height:30px"></div>X<p style="margin:0;width:20px">A</p><span id=q style="position:absolute">Q</span><p style="margin:0;width:20px">B</p>T</div>"#,
        [60.0, 20.0, 10.0, 10.0],
    ),
    (
        "center beside a right float",
        r#"<div id=c style="text-align:center;direction:ltr;width:100px"><div style="float:right;width:30px;height:30px"></div>X<p style="margin:0;width:20px">A</p><span id=q style="position:absolute">Q</span><p style="margin:0;width:20px">B</p>T</div>"#,
        [35.0, 20.0, 10.0, 10.0],
    ),
    (
        "left in rtl beside a left float",
        r#"<div id=c style="text-align:left;direction:rtl;width:100px"><div style="float:left;width:40px;height:30px"></div>X<p style="margin:0;width:20px">A</p><span id=q style="position:absolute">Q</span><p style="margin:0;width:20px">B</p>T</div>"#,
        [30.0, 20.0, 10.0, 10.0],
    ),
    (
        "right below a float wider than the line",
        r#"<div id=c style="text-align:right;direction:ltr;width:100px"><div style="float:left;width:120px;height:30px"></div>X<p style="margin:0;width:20px">A</p><span id=q style="position:absolute">Q</span><p style="margin:0;width:20px">B</p>T</div>"#,
        [100.0, 50.0, 10.0, 10.0],
    ),
    (
        "justify ignores text-align-last",
        r#"<div id=c style="text-align:justify;text-align-last:right;direction:ltr;width:100px">X<p style="margin:0;width:20px">A</p><span id=q style="position:absolute">Q</span><p style="margin:0;width:20px">B</p>T</div>"#,
        [0.0, 20.0, 10.0, 10.0],
    ),
    (
        "justify-all keeps the start in rtl",
        r#"<div id=c style="text-align:justify-all;direction:rtl;width:100px">X<p style="margin:0;width:20px">A</p><span id=q style="position:absolute">Q</span><p style="margin:0;width:20px">B</p>T</div>"#,
        [90.0, 20.0, 10.0, 10.0],
    ),
    (
        "center below a short float",
        r#"<div id=c style="text-align:center;direction:ltr;width:100px"><div style="float:left;width:40px;height:10px"></div>X<p style="margin:0;width:20px">A</p><span id=q style="position:absolute">Q</span><p style="margin:0;width:20px">B</p>T</div>"#,
        [50.0, 20.0, 10.0, 10.0],
    ),
    (
        "indent moves a centered point",
        r#"<div id=c style="text-align:center;direction:ltr;width:100px;text-indent:15px">X<p style="margin:0;width:20px">A</p><span id=q style="position:absolute">Q</span><p style="margin:0;width:20px">A</p>T</div>"#,
        [65.0, 20.0, 25.0, 10.0],
    ),
    (
        "indent in rtl moves the point left",
        r#"<div id=c style="text-align:left;direction:rtl;width:100px;text-indent:15px">X<p style="margin:0;width:20px">A</p><span id=q style="position:absolute">Q</span><p style="margin:0;width:20px">A</p>T</div>"#,
        [-40.0, 20.0, 25.0, 10.0],
    ),
    (
        "in an inline box the last line takes text-align-last",
        r#"<div id=c style="text-align:justify;text-align-last:right;direction:ltr;width:100px">X<p style="margin:0;width:20px">A</p><span><span id=q style="position:absolute">Q</span></span><p style="margin:0;width:20px">A</p>T</div>"#,
        [100.0, 20.0, 10.0, 10.0],
    ),
    (
        "in an inline box text-align-last start wins over center",
        r#"<div id=c style="text-align:center;text-align-last:start;direction:ltr;width:100px">X<p style="margin:0;width:20px">A</p><span><span id=q style="position:absolute">Q</span></span><p style="margin:0;width:20px">A</p>T</div>"#,
        [0.0, 20.0, 10.0, 10.0],
    ),
    (
        "block flow ignores text-align-last justify",
        r#"<div id=c style="text-align:right;text-align-last:justify;direction:ltr;width:100px"><p style="margin:0;width:20px">A</p><span id=q style="position:absolute">Q</span><p style="margin:0;width:20px">A</p></div>"#,
        [100.0, 10.0, 10.0, 10.0],
    ),
    (
        "block flow ignores text-align-last center in rtl",
        r#"<div id=c style="text-align:justify;text-align-last:center;direction:rtl;width:100px"><div style="float:left;width:40px;height:30px"></div><p style="margin:0;width:20px">A</p><span id=q style="position:absolute">Q</span><p style="margin:0;width:20px">A</p></div>"#,
        [90.0, 10.0, 10.0, 10.0],
    ),
    (
        "block flow justify-all keeps the start",
        r#"<div id=c style="text-align:justify-all;direction:ltr;width:100px"><p style="margin:0;width:20px">A</p><span id=q style="position:absolute">Q</span><p style="margin:0;width:20px">A</p></div>"#,
        [0.0, 10.0, 10.0, 10.0],
    ),
    (
        "center in rtl beside a float of the parent",
        r#"<div style="float:left;width:30px;height:50px"></div><div id=c style="text-align:center;direction:rtl;width:100px">X<p style="margin:0;width:20px">A</p><span id=q style="position:absolute">Q</span><p style="margin:0;width:20px">A</p>T</div>"#,
        [55.0, 20.0, 10.0, 10.0],
    ),
    (
        "negative indent",
        r#"<div id=c style="text-align:right;direction:ltr;width:100px;text-indent:-15px">X<p style="margin:0;width:20px">A</p><span id=q style="position:absolute">Q</span><p style="margin:0;width:20px">B</p>T</div>"#,
        [85.0, 20.0, 0.0, 10.0],
    ),
    (
        "hanging indent does not move the point",
        r#"<div id=c style="text-align:center;direction:ltr;width:100px;text-indent:15px hanging">X<p style="margin:0;width:20px">A</p><span id=q style="position:absolute">Q</span><p style="margin:0;width:20px">A</p>T</div>"#,
        [50.0, 20.0, 10.0, 10.0],
    ),
    (
        "block flow percentage indent",
        r#"<div id=c style="text-align:left;direction:ltr;width:100px;text-indent:20%"><p style="margin:0;width:20px">A</p><span id=q style="position:absolute">Q</span><p style="margin:0;width:20px">A</p></div>"#,
        [20.0, 10.0, 10.0, 10.0],
    ),
    (
        "hanging indent narrows a later line in an inline box",
        r#"<div id=c style="text-align:center;direction:ltr;width:100px;text-indent:15px hanging">X<p style="margin:0;width:20px">A</p><span><span id=q style="position:absolute">Q</span></span><p style="margin:0;width:20px">A</p>T</div>"#,
        [57.5, 20.0, 10.0, 10.0],
    ),
    (
        "hanging indent in rtl narrows from the right",
        r#"<div id=c style="text-align:center;direction:rtl;width:100px;text-indent:15px hanging">X<p style="margin:0;width:20px">A</p><span><span id=q style="position:absolute">Q</span></span><p style="margin:0;width:20px">A</p>T</div>"#,
        [32.5, 20.0, 10.0, 10.0],
    ),
    (
        "block flow hanging indent",
        r#"<div id=c style="text-align:left;direction:ltr;width:100px;text-indent:15px hanging"><p style="margin:0;width:20px">A</p><span id=q style="position:absolute">Q</span><p style="margin:0;width:20px">A</p></div>"#,
        [0.0, 10.0, 10.0, 10.0],
    ),
    (
        "block flow indent in rtl",
        r#"<div id=c style="text-align:right;direction:rtl;width:100px;text-indent:15px"><p style="margin:0;width:20px">A</p><span id=q style="position:absolute">Q</span><p style="margin:0;width:20px">A</p></div>"#,
        [60.0, 10.0, 25.0, 10.0],
    ),
    (
        "each-line indent after a forced break",
        r#"<div id=c style="text-align:center;width:100px;text-indent:15px each-line">X<p style="margin:0;width:20px">A</p>Y<br><span id=q style="position:absolute">Q</span><p style="margin:0;width:20px">A</p>T</div>"#,
        [57.5, 30.0, 25.0, 10.0],
    ),
    (
        "indent skips the line after a forced break in rtl",
        r#"<div id=c style="text-align:right;direction:rtl;width:100px;text-indent:15px">X<p style="margin:0;width:20px">A</p>Y<br><span id=q style="position:absolute">Q</span><p style="margin:0;width:20px">A</p>T</div>"#,
        [75.0, 30.0, 25.0, 10.0],
    ),
    (
        "hanging indent on the line after a forced break",
        r#"<div id=c style="text-align:center;width:100px;text-indent:15px hanging">X<p style="margin:0;width:20px">A</p>Y<br><span id=q style="position:absolute">Q</span><p style="margin:0;width:20px">A</p>T</div>"#,
        [57.5, 30.0, 10.0, 10.0],
    ),
    (
        "the line after a forced break takes text-align-last",
        r#"<div id=c style="text-align:justify;text-align-last:center;width:100px">X<p style="margin:0;width:20px">A</p>Y<br><span id=q style="position:absolute">Q</span><p style="margin:0;width:20px">A</p>T</div>"#,
        [50.0, 30.0, 10.0, 10.0],
    ),
    (
        "a float after the box does not narrow its line",
        r#"<div style="text-align:center;width:100px">X<p style="margin:0;width:20px">A</p><span id=q style="position:absolute">Q</span><div style="float:left;width:40px;height:20px"></div><p style="margin:0;width:20px">A</p>T</div>"#,
        [50.0, 20.0, 10.0, 10.0],
    ),
    (
        "in a ruby box the last line takes text-align-last",
        r#"<div style="text-align:justify;text-align-last:right;width:100px">X<p style="margin:0;width:20px">A</p><ruby><span id=q style="position:absolute">Q</span></ruby><p style="margin:0;width:20px">A</p>T</div>"#,
        [100.0, 20.0, 10.0, 10.0],
    ),
    (
        "a line after an empty block is not the first line",
        r#"<div style="text-indent:15px;text-align:center;width:100px"><div></div><span><span id=q style="position:absolute">Q</span></span><p style="margin:0;width:20px">A</p>T</div>"#,
        [50.0, 0.0, 25.0, 10.0],
    ),
    (
        "in an inline box text-align-last end wins over center",
        r#"<div style="text-align:center;text-align-last:end;width:100px">X<p style="margin:0;width:20px">A</p><span><span id=q style="position:absolute">Q</span></span><p style="margin:0;width:20px">A</p>T</div>"#,
        [100.0, 20.0, 10.0, 10.0],
    ),
    (
        "in an inline box text-align-last left wins over center",
        r#"<div style="text-align:center;text-align-last:left;width:100px">X<p style="margin:0;width:20px">A</p><span><span id=q style="position:absolute">Q</span></span><p style="margin:0;width:20px">A</p>T</div>"#,
        [0.0, 20.0, 10.0, 10.0],
    ),
    (
        "a float in a later block does not narrow the line",
        r#"<div style="width:100px">X<p style="margin:0">A</p><span id=q style="position:absolute">Q</span><div><div style="float:left;width:40px;height:20px"></div>B</div>T</div>"#,
        [0.0, 20.0, 10.0, 10.0],
    ),
];

#[test]
fn an_inline_box_on_a_line_of_its_own_takes_the_chromium_static_position() {
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
