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
//! A box that follows a block-level sibling and comes before a float, with
//! no line content between, stays out of the anonymous block that holds the
//! line beside the float, so it sits on such an empty line too.
//!
//! A block-level `::before` or `::after` of an element in the paragraph is
//! placed in document order with the block children: a box without an anchor
//! sits where the first block-level box after it starts, or at the end of the
//! content when none follows. A run of children that holds an inline element,
//! even one split by such a block, gets a line like the others, so the box
//! there is aligned as a last line.
//!
//! The same holds where block flow lays out the children, for a container
//! whose inline children hold no text: a box joins the anonymous block of the
//! inline elements before it since the last block-level box, or else of the
//! ones after it unless a float comes between, and a box inside an inline
//! element is on a line of the paragraph around that element. Such a line
//! is aligned as a last line, and the indent narrows it only when nothing
//! block-level comes before it in the paragraph.
//!
//! The content of an anonymous table cell is a paragraph like any other:
//! a box there takes the same static position, inside the cell.
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
    (
        "an empty block between a forced break and the box keeps the line empty",
        r#"<div style="text-align:left;text-align-last:right;width:100px">X<br><div></div><span id=q style="position:absolute">Q</span><p style="margin:0">A</p>T</div>"#,
        [0.0, 10.0, 10.0, 10.0],
    ),
    (
        "an empty block between a forced break and the box in rtl",
        r#"<div style="direction:rtl;text-align:right;text-align-last:left;width:100px">X<br><div></div><span id=q style="position:absolute">Q</span><p style="margin:0">A</p>T</div>"#,
        [90.0, 10.0, 10.0, 10.0],
    ),
    (
        "a forced break after an empty block still takes text-align-last",
        r#"<div style="text-align:left;text-align-last:right;width:100px"><div></div>X<br><span id=q style="position:absolute">Q</span><p style="margin:0">A</p>T</div>"#,
        [100.0, 10.0, 10.0, 10.0],
    ),
    (
        "a box after the last block keeps the floats beside it",
        r#"<div style="text-align:center;width:100px"><div style="float:left;width:40px;height:30px"></div>X<div></div><span id=q style="position:absolute">Q</span></div>"#,
        [70.0, 10.0, 10.0, 10.0],
    ),
    (
        "a box after the last block keeps the floats beside it in rtl",
        r#"<div style="text-align:right;direction:rtl;width:100px"><div style="float:right;width:40px;height:30px"></div>X<p style="margin:0">B</p><span id=q style="position:absolute">Q</span></div>"#,
        [50.0, 20.0, 10.0, 10.0],
    ),
    (
        "of empty forced-break lines at one offset the last one counts",
        r#"<div style="text-align:left;text-align-last:right;line-height:0;width:100px">X<br><div></div><br><span id=q style="position:absolute;line-height:10px">Q</span><p style="margin:0;line-height:10px">A</p>T</div>"#,
        [100.0, 0.0, 10.0, 10.0],
    ),
    (
        "an empty block after empty forced-break lines keeps the line empty",
        r#"<div style="text-align:left;text-align-last:right;line-height:0;width:100px">X<br><br><div></div><span id=q style="position:absolute;line-height:10px">Q</span><p style="margin:0;line-height:10px">A</p>T</div>"#,
        [0.0, 0.0, 10.0, 10.0],
    ),
    (
        "a forced break after the box and a later block does not count",
        r#"<div style="text-align:left;text-align-last:right;line-height:0;width:100px"><span id=q style="position:absolute;line-height:10px">Q</span><div></div>X<br></div>"#,
        [0.0, 0.0, 10.0, 10.0],
    ),
    (
        "a forced break after the box and a later block does not count in rtl",
        r#"<div style="direction:rtl;text-align:right;text-align-last:left;line-height:0;width:100px"><span id=q style="position:absolute;line-height:10px">Q</span><div></div>X<br></div>"#,
        [90.0, 0.0, 10.0, 10.0],
    ),
    (
        "a forced break before the box still counts with another after it",
        r#"<div style="text-align:left;text-align-last:right;line-height:0;width:100px">Y<br><span id=q style="position:absolute;line-height:10px">Q</span><div></div>X<br></div>"#,
        [100.0, 0.0, 10.0, 10.0],
    ),
    (
        "a forced break in a block-level ::after does not count",
        r#"<style>#c::after{display:block;content:"X\A";white-space:pre}</style><div id=c style="text-align:left;text-align-last:right;line-height:0;width:100px"><span id=q style="position:absolute;line-height:10px">Q</span></div>"#,
        [0.0, 0.0, 10.0, 10.0],
    ),
    (
        "a forced break in a block-level ::before does not count",
        r#"<style>#c::before{display:block;content:"X\A";white-space:pre}</style><div id=c style="text-align:left;text-align-last:right;line-height:0;width:100px"><span id=q style="position:absolute;line-height:10px">Q</span></div>"#,
        [0.0, 0.0, 10.0, 10.0],
    ),
    (
        "a forced break in ::after after a block does not count",
        r#"<style>#c::after{display:block;content:"X\A";white-space:pre}</style><div id=c style="text-align:left;text-align-last:right;line-height:0;width:100px">Y<p style="margin:0">A</p><span id=q style="position:absolute;line-height:10px">Q</span></div>"#,
        [0.0, 0.0, 10.0, 10.0],
    ),
    (
        "a forced break after a block-level ::before counts",
        r#"<style>#c::before{display:block;content:"X\A";white-space:pre}</style><div id=c style="text-align:left;text-align-last:right;line-height:0;width:100px">Y<br><span id=q style="position:absolute;line-height:10px">Q</span><p style="margin:0">A</p></div>"#,
        [100.0, 0.0, 10.0, 10.0],
    ),
    (
        "a forced break before a block-level ::after counts",
        r#"<style>#c::after{display:block;content:"X\A";white-space:pre}</style><div id=c style="text-align:left;text-align-last:right;width:100px">Y<br><span id=q style="position:absolute">Q</span></div>"#,
        [100.0, 10.0, 10.0, 10.0],
    ),
    (
        "a block-level ::before alone before the box does not count",
        r#"<style>#c::before{display:block;content:"X\A";white-space:pre}</style><div id=c style="text-align:left;text-align-last:right;width:100px"><span id=q style="position:absolute">Q</span><p style="margin:0">A</p>T</div>"#,
        [0.0, 10.0, 10.0, 10.0],
    ),
    (
        "a forced break before the box counts with a generated block and a break after it",
        r#"<style>.b::before{display:block;content:"Z"}</style><div style="text-align:left;text-align-last:right;width:100px"><p style="margin:0">A</p>Y<br><span id=q style="position:absolute">Q</span><span class=b></span>W<br></div>"#,
        [100.0, 20.0, 10.0, 10.0],
    ),
    (
        "a line after an empty forced-break line is not the first line",
        r#"<div style="text-align:left;text-indent:15px;line-height:0;width:100px">X<br><span id=q style="position:absolute;line-height:10px">Q</span><div></div></div>"#,
        [0.0, 0.0, 25.0, 10.0],
    ),
    (
        "a line after an empty forced-break line is not the first line in rtl",
        r#"<div style="direction:rtl;text-align:right;text-indent:15px;line-height:0;width:100px">X<br><span id=q style="position:absolute;line-height:10px">Q</span><div></div></div>"#,
        [75.0, 0.0, 25.0, 10.0],
    ),
    (
        "an inline box that starts the paragraph is on the first line",
        r#"<div style="text-align:left;text-indent:15px;line-height:0;width:100px"><span><span id=q style="position:absolute;line-height:10px">Q</span></span><div></div>X</div>"#,
        [15.0, 0.0, 25.0, 10.0],
    ),
    (
        "an inline box after a block-level ::before is not on the first line",
        r#"<style>#c::before{display:block;content:"";height:20px}</style><div id=c style="text-align:left;text-indent:15px;width:100px"><span><span id=q style="position:absolute">Q</span></span><div></div>X</div>"#,
        [0.0, 20.0, 25.0, 10.0],
    ),
    (
        "an inline box after an empty block-level ::before is not on the first line",
        r#"<style>#c::before{display:block;content:""}</style><div id=c style="text-indent:15px;width:100px"><span><span id=q style="position:absolute">Q</span></span><div></div>X</div>"#,
        [0.0, 0.0, 25.0, 10.0],
    ),
    (
        "an empty line after an empty block-level ::before keeps the indent shift",
        r#"<style>#c::before{display:block;content:""}</style><div id=c style="text-indent:15px;text-align:left;width:100px"><span id=q style="position:absolute">Q</span><div></div>X</div>"#,
        [15.0, 0.0, 25.0, 10.0],
    ),
    (
        "a box inside an element after its block-level ::before is not on the first line",
        r#"<style>.b::before{display:block;content:"";height:20px}</style><div style="text-align:left;text-indent:15px;width:100px"><span class=b><span id=q style="position:absolute">Q</span></span></div>"#,
        [0.0, 20.0, 25.0, 10.0],
    ),
    (
        "a box after an earlier element's block-level ::before is not on the first line",
        r#"<style>.b::before{display:block;content:"";height:20px}</style><div style="text-align:left;text-indent:15px;width:100px"><span class=b></span><span><span id=q style="position:absolute">Q</span></span></div>"#,
        [0.0, 20.0, 25.0, 10.0],
    ),
    (
        "a box after an earlier element's block-level ::after is not on the first line",
        r#"<style>.a::after{display:block;content:"";height:20px}</style><div style="text-align:left;text-indent:15px;width:100px"><span class=a></span><span><span id=q style="position:absolute">Q</span></span></div>"#,
        [0.0, 20.0, 25.0, 10.0],
    ),
    (
        "a box deep inside an element after its block-level ::before in rtl",
        r#"<style>.b::before{display:block;content:"";height:20px}</style><div style="text-align:left;text-indent:15px;direction:rtl;width:100px"><span class=b><span><span id=q style="position:absolute">Q</span></span></span></div>"#,
        [-25.0, 20.0, 25.0, 10.0],
    ),
    (
        "a box after a block-level ::before in an anonymous table cell",
        r#"<style>.b::before{display:block;content:"";height:20px}</style><div style="display:table-row;text-indent:15px;text-align:left"><span class=b><span id=q style="position:absolute">Q</span></span></div>"#,
        [0.0, 20.0, 25.0, 10.0],
    ),
    (
        "center before a float after a block",
        r#"<div style="text-align:center;direction:ltr;width:100px">X<p style="margin:0;width:20px">A</p><span id=q style="position:absolute">Q</span><div style="float:left;width:40px;height:20px"></div>T</div>"#,
        [50.0, 20.0, 10.0, 10.0],
    ),
    (
        "left in rtl before a right float after a block",
        r#"<div style="text-align:left;direction:rtl;width:100px">X<p style="margin:0;width:20px">A</p><span id=q style="position:absolute">Q</span><div style="float:right;width:40px;height:20px"></div>TT UU</div>"#,
        [-10.0, 20.0, 10.0, 10.0],
    ),
    (
        "right before a float after a block",
        r#"<div style="text-align:right;direction:ltr;width:100px">X<p style="margin:0;width:20px">A</p><span id=q style="position:absolute">Q</span><div style="float:left;width:40px;height:20px"></div>T</div>"#,
        [100.0, 20.0, 10.0, 10.0],
    ),
    (
        "center after a float after a block keeps its anchor",
        r#"<div style="text-align:center;direction:ltr;width:100px">X<p style="margin:0;width:20px">A</p><div style="float:left;width:40px;height:20px"></div><span id=q style="position:absolute">Q</span>T</div>"#,
        [65.0, 20.0, 10.0, 10.0],
    ),
    (
        "right in rtl before a float after a block and white space",
        r#"<div style="text-align:right;direction:rtl;width:100px">X<p style="margin:0;width:20px">A</p> <span id=q style="position:absolute">Q</span><div style="float:left;width:40px;height:20px"></div>T</div>"#,
        [90.0, 20.0, 10.0, 10.0],
    ),
    (
        "center between floats after a block",
        r#"<div style="text-align:center;direction:ltr;width:100px">X<p style="margin:0;width:20px">A</p><div style="float:left;width:40px;height:20px"></div><span id=q style="position:absolute">Q</span><div style="float:right;width:40px;height:20px"></div>T</div>"#,
        [70.0, 20.0, 10.0, 10.0],
    ),
    (
        "center in rtl between floats after a block",
        r#"<div style="text-align:center;direction:rtl;width:100px">X<p style="margin:0;width:20px">A</p><div style="float:left;width:40px;height:20px"></div><span id=q style="position:absolute">Q</span><div style="float:right;width:40px;height:20px"></div>T</div>"#,
        [60.0, 20.0, 10.0, 10.0],
    ),
    (
        "center in display contents before a float after a block",
        r#"<div style="text-align:center;direction:ltr;width:100px">X<p style="margin:0;width:20px">A</p><span style="display:contents"><span id=q style="position:absolute">Q</span></span><div style="float:left;width:40px;height:20px"></div>T</div>"#,
        [50.0, 20.0, 10.0, 10.0],
    ),
    (
        "right before a float in display contents after a block",
        r#"<div style="text-align:right;direction:ltr;width:100px">X<p style="margin:0;width:20px">A</p><span id=q style="position:absolute">Q</span><span style="display:contents"><div style="float:left;width:40px;height:20px"></div></span>T</div>"#,
        [100.0, 20.0, 10.0, 10.0],
    ),
    (
        "center before a hidden element and a float after a block",
        r#"<div style="text-align:center;direction:ltr;width:100px">X<p style="margin:0;width:20px">A</p><span id=q style="position:absolute">Q</span><b style="display:none">Z</b><div style="float:left;width:40px;height:20px"></div>T</div>"#,
        [50.0, 20.0, 10.0, 10.0],
    ),
    (
        "center before an empty span and a float keeps its anchor",
        r#"<div style="text-align:center;direction:ltr;width:100px">X<p style="margin:0;width:20px">A</p><span id=q style="position:absolute">Q</span><span></span><div style="float:left;width:40px;height:20px"></div>T</div>"#,
        [65.0, 20.0, 10.0, 10.0],
    ),
    (
        "center before a float after a block with a bottom margin",
        r#"<div style="text-align:center;direction:ltr;width:100px">X<p style="margin:0 0 10px;width:20px">A</p><span id=q style="position:absolute">Q</span><div style="float:left;width:40px;height:20px"></div>T</div>"#,
        [50.0, 30.0, 10.0, 10.0],
    ),
    (
        "center before a float after an empty block",
        r#"<div style="text-align:center;direction:ltr;width:100px"><div></div><span id=q style="position:absolute">Q</span><div style="float:left;width:40px;height:20px"></div>T</div>"#,
        [50.0, 0.0, 10.0, 10.0],
    ),
    (
        "center before a float at the start keeps its anchor",
        r#"<div style="text-align:center;direction:ltr;width:100px;display:flow-root"><span id=q style="position:absolute">Q</span><div style="float:left;width:40px;height:20px"></div>T</div>"#,
        [65.0, 0.0, 10.0, 10.0],
    ),
    (
        "center in rtl before a float after a block-level ::before",
        r#"<style>.bb::before{content:"B";display:block}</style><div class=bb style="text-align:center;direction:rtl;width:100px"><span id=q style="position:absolute">Q</span><div style="float:left;width:40px;height:20px"></div>T</div>"#,
        [40.0, 10.0, 10.0, 10.0],
    ),
    (
        "indented before a float after a block",
        r#"<div style="text-align:left;direction:ltr;width:100px;text-indent:15px">X<p style="margin:0;width:20px">A</p><span id=q style="position:absolute">Q</span><div style="float:left;width:40px;height:20px"></div>T</div>"#,
        [15.0, 20.0, 25.0, 10.0],
    ),
    (
        "left before a float too wide to share the line after a block",
        r#"<div style="text-align:left;width:100px">X<p style="margin:0;width:20px">A</p><span id=q style="position:absolute">Q</span><div style="float:left;width:100px;height:20px"></div>T</div>"#,
        [0.0, 20.0, 10.0, 10.0],
    ),
    (
        "center before a float too wide to share the line after an anonymous table",
        r#"<div style="text-align:center;width:100px"><span style="display:table-cell">C</span><span id=q style="position:absolute">Q</span><div style="float:left;width:100px;height:20px"></div>T</div>"#,
        [50.0, 10.0, 10.0, 10.0],
    ),
    (
        "center after generated text in display contents keeps its anchor",
        r#"<style>.t::before{content:"Z"}</style><div style="text-align:center;width:100px">X<p style="margin:0;width:20px">A</p><span class=t style="display:contents"></span><span id=q style="position:absolute">Q</span><div style="float:left;width:40px;height:20px"></div>T</div>"#,
        [70.0, 20.0, 10.0, 10.0],
    ),
    (
        "left in an anonymous cell after a cell keeps its anchor",
        r#"<div style="text-align:left;width:100px"><div style="display:table-row"><div style="display:table-cell">C</div><span id=q style="position:absolute">Q</span><div style="float:left;width:40px;height:20px"></div>T<p style="margin:0">B</p></div></div>"#,
        [50.0, 0.0, 10.0, 10.0],
    ),
    (
        "center before a float in display contents with generated text after",
        r#"<style>.t::after{content:"Z"}</style><div style="text-align:center;width:100px">X<p style="margin:0;width:20px">A</p><span class=t style="display:contents"><span id=q style="position:absolute">Q</span><div style="float:left;width:40px;height:20px"></div></span>T</div>"#,
        [50.0, 20.0, 10.0, 10.0],
    ),
    (
        "center before generated text and a float keeps its anchor",
        r#"<style>.t::after{content:"Z"}</style><div style="text-align:center;width:100px">X<p style="margin:0;width:20px">A</p><span class=t style="display:contents"><span id=q style="position:absolute">Q</span></span><div style="float:left;width:40px;height:20px"></div>T</div>"#,
        [60.0, 20.0, 10.0, 10.0],
    ),
    (
        "center before a too wide float after a block-level ::before",
        r#"<style>.bb::before{content:"B";display:block}</style><div class=bb style="text-align:center;width:100px"><span id=q style="position:absolute">Q</span><div style="float:left;width:100px;height:20px"></div>T</div>"#,
        [50.0, 10.0, 10.0, 10.0],
    ),
    (
        "center before a too wide float after a padded block-level ::before",
        r#"<style>.bb::before{content:"B";display:block;padding:3px}</style><div class=bb style="text-align:center;width:100px"><span id=q style="position:absolute">Q</span><div style="float:left;width:100px;height:20px"></div>T</div>"#,
        [50.0, 16.0, 10.0, 10.0],
    ),
    (
        "center before a too wide float after a block-level ::before with a margin",
        r#"<style>.bb::before{content:"B";display:block;margin-bottom:10px}</style><div class=bb style="text-align:center;width:100px"><span id=q style="position:absolute">Q</span><div style="float:left;width:100px;height:20px"></div>T</div>"#,
        [50.0, 20.0, 10.0, 10.0],
    ),
    (
        "center before a comment and a float after a block",
        r#"<div style="text-align:center;width:100px">X<p style="margin:0;width:20px">A</p><span id=q style="position:absolute">Q</span><!-- c --><div style="float:left;width:40px;height:20px"></div>T</div>"#,
        [50.0, 20.0, 10.0, 10.0],
    ),
    (
        "center before a float after the block-level ::before of display contents",
        r#"<style>.t::before{content:"Z";display:block}</style><div style="text-align:center;width:100px">X<span class=t style="display:contents"><span id=q style="position:absolute">Q</span><div style="float:left;width:40px;height:20px"></div></span>T</div>"#,
        [50.0, 20.0, 10.0, 10.0],
    ),
    (
        "last-line aligned before an element block ::before at the start",
        r#"<style>.b::before{display:block;content:"Z"}.a::after{display:block;content:"Y"}.m::before{display:block;content:"Z";margin:3px 0 7px}.n::after{display:block;content:"Y";margin:3px 0 7px}</style><div id=c style="text-align:left;text-align-last:right;width:100px"><span id=q style="position:absolute">Q</span><span class=b></span>W<br></div>"#,
        [100.0, 0.0, 10.0, 10.0],
    ),
    (
        "last-line aligned after an element block ::before, before a block",
        r#"<style>.b::before{display:block;content:"Z"}.a::after{display:block;content:"Y"}.m::before{display:block;content:"Z";margin:3px 0 7px}.n::after{display:block;content:"Y";margin:3px 0 7px}</style><div id=c style="text-align:left;text-align-last:right;width:100px"><span class=b></span><span id=q style="position:absolute">Q</span><p style="margin:0">A</p>T<br></div>"#,
        [100.0, 10.0, 10.0, 10.0],
    ),
    (
        "before an element block ::after with nothing else",
        r#"<style>.b::before{display:block;content:"Z"}.a::after{display:block;content:"Y"}.m::before{display:block;content:"Z";margin:3px 0 7px}.n::after{display:block;content:"Y";margin:3px 0 7px}</style><div id=c style="text-align:left;text-align-last:right;width:100px"><span id=q style="position:absolute">Q</span><span class=a></span></div>"#,
        [100.0, 0.0, 10.0, 10.0],
    ),
    (
        "after a block and an element block ::after at the end",
        r#"<style>.b::before{display:block;content:"Z"}.a::after{display:block;content:"Y"}.m::before{display:block;content:"Z";margin:3px 0 7px}.n::after{display:block;content:"Y";margin:3px 0 7px}</style><div id=c style="text-align:left;text-align-last:right;width:100px"><p style="margin:0">A</p><span class=a></span><span id=q style="position:absolute">Q</span></div>"#,
        [100.0, 20.0, 10.0, 10.0],
    ),
    (
        "right-aligned paragraph after an element block ::after, before a block",
        r#"<style>.b::before{display:block;content:"Z"}.a::after{display:block;content:"Y"}.m::before{display:block;content:"Z";margin:3px 0 7px}.n::after{display:block;content:"Y";margin:3px 0 7px}</style><div id=c style="text-align:right;text-align-last:left;width:100px"><span class=a></span><span id=q style="position:absolute">Q</span><p style="margin:0">A</p></div>"#,
        [0.0, 10.0, 10.0, 10.0],
    ),
    (
        "indented first line before an element block ::before holding text",
        r#"<style>.b::before{display:block;content:"Z"}.a::after{display:block;content:"Y"}.m::before{display:block;content:"Z";margin:3px 0 7px}.n::after{display:block;content:"Y";margin:3px 0 7px}</style><div id=c style="text-indent:15px;text-align:center;width:100px"><span id=q style="position:absolute">Q</span><span class=b>M</span><p style="margin:0">A</p></div>"#,
        [57.5, 0.0, 25.0, 10.0],
    ),
    (
        "after an element block ::before with margins at the end, centered",
        r#"<style>.b::before{display:block;content:"Z"}.a::after{display:block;content:"Y"}.m::before{display:block;content:"Z";margin:3px 0 7px}.n::after{display:block;content:"Y";margin:3px 0 7px}</style><div id=c style="text-indent:15px;text-align:center;width:100px"><div style="border-top:1px solid"></div><span class=m></span><span id=q style="position:absolute">Q</span></div>"#,
        [50.0, 21.0, 25.0, 10.0],
    ),
    (
        "between a block, an element block ::before and another, not indented",
        r#"<style>.b::before{display:block;content:"Z"}.a::after{display:block;content:"Y"}.m::before{display:block;content:"Z";margin:3px 0 7px}.n::after{display:block;content:"Y";margin:3px 0 7px}</style><div id=c style="text-indent:15px;width:100px"><p style="margin:0">A</p><span class=b></span><span id=q style="position:absolute">Q</span><span class=b></span>W</div>"#,
        [0.0, 20.0, 25.0, 10.0],
    ),
    (
        "after two element block ::before boxes and before a third",
        r#"<style>.b::before{display:block;content:"Z"}.a::after{display:block;content:"Y"}.m::before{display:block;content:"Z";margin:3px 0 7px}</style><div id=c style="text-align:center;width:100px"><span class=b></span><span class=b></span><span id=q style="position:absolute">Q</span><span class=b></span>W</div>"#,
        [50.0, 20.0, 10.0, 10.0],
    ),
    (
        "between an element block ::before and a block after a block",
        r#"<style>.b::before{display:block;content:"Z"}.a::after{display:block;content:"Y"}.m::before{display:block;content:"Z";margin:3px 0 7px}</style><div id=c style="text-align:center;width:100px"><p style="margin:0">A</p><span class=b></span><span id=q style="position:absolute">Q</span><p style="margin:0">B</p></div>"#,
        [50.0, 20.0, 10.0, 10.0],
    ),
    (
        "inside an element before its block ::after",
        r#"<style>.b::before{display:block;content:"Z"}.a::after{display:block;content:"Y"}.m::before{display:block;content:"Z";margin:3px 0 7px}</style><div id=c style="text-align:center;width:100px"><span class=a><span id=q style="position:absolute">Q</span></span>W</div>"#,
        [50.0, 0.0, 10.0, 10.0],
    ),
    (
        "after an element block ::before with margins, before a block",
        r#"<style>.b::before{display:block;content:"Z"}.a::after{display:block;content:"Y"}.m::before{display:block;content:"Z";margin:3px 0 7px}</style><div id=c style="text-align:left;width:100px"><span class=m></span><span id=q style="position:absolute">Q</span><p style="margin:0">A</p></div>"#,
        [0.0, 20.0, 10.0, 10.0],
    ),
    (
        "between element block margins and a block with margins",
        r#"<style>.b::before{display:block;content:"Z"}.a::after{display:block;content:"Y"}.m::before{display:block;content:"Z";margin:3px 0 7px}</style><div id=c style="text-align:center;width:100px"><p style="margin:0 0 6px">A</p><span class=m></span><span id=q style="position:absolute">Q</span><p style="margin:4px 0">B</p></div>"#,
        [50.0, 33.0, 10.0, 10.0],
    ),
    (
        "after a forced break following an element block ::before",
        r#"<style>.b::before{display:block;content:"Z"}.a::after{display:block;content:"Y"}.m::before{display:block;content:"Z";margin:3px 0 7px}</style><div id=c style="text-align:left;width:100px"><span class=b></span>V<br><span id=q style="position:absolute">Q</span><p style="margin:0">A</p></div>"#,
        [0.0, 20.0, 10.0, 10.0],
    ),
    (
        "inside an element before its block ::after, before a block",
        r#"<style>.b::before{display:block;content:"Z"}.a::after{display:block;content:"Y"}.m::before{display:block;content:"Z";margin:3px 0 7px}</style><div id=c style="text-align:right;width:100px"><span class=a><span id=q style="position:absolute">Q</span></span><p style="margin:0">A</p></div>"#,
        [100.0, 0.0, 10.0, 10.0],
    ),
    (
        "between two element block ::before boxes with margins",
        r#"<style>.b::before{display:block;content:"Z"}.a::after{display:block;content:"Y"}.m::before{display:block;content:"Z";margin:3px 0 7px}</style><div id=c style="text-align:right;width:100px"><span class=m></span><span id=q style="position:absolute">Q</span><span class=m></span>W</div>"#,
        [100.0, 20.0, 10.0, 10.0],
    ),
    (
        "after an element block ::after with a bottom margin at the end",
        r#"<style>.b::before{display:block;content:"Z"}.a::after{display:block;content:"Y"}.m::before{display:block;content:"Z";margin:3px 0 7px}.n::after{display:block;content:"Y";margin:3px 0 7px}</style><div id=c style="text-align:left;width:100px"><span class=n></span><span id=q style="position:absolute">Q</span></div>"#,
        [0.0, 20.0, 10.0, 10.0],
    ),
    (
        "after a block margin and an element block ::before at the end",
        r#"<style>.b::before{display:block;content:"Z"}.a::after{display:block;content:"Y"}.m::before{display:block;content:"Z";margin:3px 0 7px}.n::after{display:block;content:"Y";margin:3px 0 7px}</style><div id=c style="text-align:left;width:100px"><p style="margin:0 0 5px">A</p><span class=b></span><span id=q style="position:absolute">Q</span></div>"#,
        [0.0, 25.0, 10.0, 10.0],
    ),
    (
        "after an element holding a block and its block ::after",
        r#"<style>.b::before{display:block;content:"Z"}.a::after{display:block;content:"Y"}.m::before{display:block;content:"Z";margin:3px 0 7px}.n::after{display:block;content:"Y";margin:3px 0 7px}</style><div id=c style="text-align:left;width:100px"><span class=a><p style="margin:0">A</p></span><span id=q style="position:absolute">Q</span></div>"#,
        [0.0, 20.0, 10.0, 10.0],
    ),
    (
        "before an element block ::before that holds a block",
        r#"<style>.b::before{display:block;content:"Z"}.a::after{display:block;content:"Y"}.m::before{display:block;content:"Z";margin:3px 0 7px}.n::after{display:block;content:"Y";margin:3px 0 7px}</style><div id=c style="text-align:right;width:100px"><span id=q style="position:absolute">Q</span><span class=b><p style="margin:0">A</p></span></div>"#,
        [100.0, 0.0, 10.0, 10.0],
    ),
    (
        "after an element block ::after with margins, before a block with margins",
        r#"<style>.b::before{display:block;content:"Z"}.a::after{display:block;content:"Y"}.m::before{display:block;content:"Z";margin:3px 0 7px}.n::after{display:block;content:"Y";margin:3px 0 7px}</style><div id=c style="text-align:left;width:100px"><span class=n></span><span id=q style="position:absolute">Q</span><p style="margin:0 0 5px">A</p></div>"#,
        [0.0, 20.0, 10.0, 10.0],
    ),
    (
        "after an element block ::before, no first-line indent",
        r#"<style>.b::before{display:block;content:"Z"}.a::after{display:block;content:"Y"}</style><div id=c style="text-indent:15px;width:100px"><span class=b></span><span id=q style="position:absolute">Q</span></div>"#,
        [0.0, 10.0, 25.0, 10.0],
    ),
    (
        "after a nested element block ::before",
        r#"<style>.b::before{display:block;content:"Z"}.a::after{display:block;content:"Y"}</style><div id=c style="text-align:left;text-align-last:right;width:100px"><span><span class=b></span></span><span id=q style="position:absolute">Q</span></div>"#,
        [100.0, 10.0, 10.0, 10.0],
    ),
    (
        "between blocks after an element block ::before, empty line",
        r#"<style>.b::before{display:block;content:"Z"}.a::after{display:block;content:"Y"}</style><div id=c style="text-indent:15px;text-align:center;width:100px"><span class=b></span><p style="margin:0">A</p><span id=q style="position:absolute">Q</span><p style="margin:0">B</p></div>"#,
        [65.0, 20.0, 25.0, 10.0],
    ),
    (
        "after an element block ::before and an anonymous table, before a block",
        r#"<style>.b::before{display:block;content:"Z"}</style><div id=c style="width:100px"><span class=b></span><span style="display:table-cell">C</span><span id=q style="position:absolute">Q</span><p style="margin:0">A</p></div>"#,
        [0.0, 20.0, 10.0, 10.0],
    ),
    (
        "after an element block ::after and an anonymous table at the end",
        r#"<style>.a::after{display:block;content:"Y"}</style><div id=c style="width:100px;text-align:left;text-align-last:right"><span class=a></span><span style="display:table-cell">C</span><span id=q style="position:absolute">Q</span></div>"#,
        [0.0, 20.0, 10.0, 10.0],
    ),
    (
        "after the lines of an element block ::before with a bottom margin",
        r#"<style>.b::before{display:block;content:"Z";margin-bottom:5px}</style><div id=c style="width:100px;text-align:left;text-align-last:right"><span class=b>MM<br></span><span id=q style="position:absolute">Q</span><p style="margin:0">A</p></div>"#,
        [100.0, 25.0, 10.0, 10.0],
    ),
    (
        "start of an anonymous table cell, before a block",
        r#"<div id=c style="display:table;width:100px"><span id=q style="position:absolute">Q</span><p style="margin:0">B</p></div>"#,
        [0.0, 0.0, 10.0, 10.0],
    ),
    (
        "block flow, an empty inline element starts the run",
        r#"<div id=c style="text-indent:15px;text-align:center;text-align-last:right;width:100px"><span></span><span id=q style="position:absolute">Q</span><p style="margin:0">A</p></div>"#,
        [100.0, 0.0, 25.0, 10.0],
    ),
    (
        "block flow, an empty inline element after a block",
        r#"<div id=c style="text-indent:15px;text-align:left;text-align-last:right;width:100px"><p style="margin:0">B</p><span></span><span id=q style="position:absolute">Q</span><p style="margin:0">A</p></div>"#,
        [100.0, 10.0, 25.0, 10.0],
    ),
    (
        "block flow, an empty inline element after the last block",
        r#"<div id=c style="text-align:left;text-align-last:right;width:100px"><p style="margin:0">B</p><span></span><span id=q style="position:absolute">Q</span></div>"#,
        [100.0, 10.0, 10.0, 10.0],
    ),
    (
        "block flow, before an inline element split by a block",
        r#"<div id=c style="text-align:left;text-align-last:right;width:100px"><span id=q style="position:absolute">Q</span><span><p style="margin:0">A</p></span></div>"#,
        [100.0, 0.0, 10.0, 10.0],
    ),
    (
        "block flow, after an inline element split by a block",
        r#"<div id=c style="text-align:left;text-align-last:right;width:100px"><span><p style="margin:0">A</p></span><span id=q style="position:absolute">Q</span></div>"#,
        [100.0, 10.0, 10.0, 10.0],
    ),
    (
        "block flow, after an empty inline element at the start",
        r#"<div id=c style="text-align:left;text-align-last:right;width:100px"><span></span><span id=q style="position:absolute">Q</span><p style="margin:0">A</p></div>"#,
        [100.0, 0.0, 10.0, 10.0],
    ),
    (
        "block flow, between a block and a split inline element",
        r#"<div id=c style="text-align:left;text-align-last:right;width:100px"><p style="margin:0">A</p><span id=q style="position:absolute">Q</span><span><p style="margin:0">B</p></span></div>"#,
        [100.0, 10.0, 10.0, 10.0],
    ),
    (
        "block flow, in an inline element after its block",
        r#"<div id=c style="text-align:left;text-align-last:right;width:100px"><span><p style="margin:0">A</p><span id=q style="position:absolute">Q</span></span></div>"#,
        [100.0, 10.0, 10.0, 10.0],
    ),
    (
        "block flow, in an inline element before its block",
        r#"<div id=c style="text-align:left;text-align-last:right;width:100px"><span><span id=q style="position:absolute">Q</span><p style="margin:0">A</p></span></div>"#,
        [100.0, 0.0, 10.0, 10.0],
    ),
    (
        "block flow, in an inline element between blocks",
        r#"<div id=c style="text-align:left;text-align-last:right;width:100px"><p style="margin:0">A</p><span><span id=q style="position:absolute">Q</span></span><p style="margin:0">B</p></div>"#,
        [100.0, 10.0, 10.0, 10.0],
    ),
    (
        "block flow, indent narrows a centered first run",
        r#"<div id=c style="text-indent:15px;text-align:center;width:100px"><span></span><span id=q style="position:absolute">Q</span><p style="margin:0">A</p></div>"#,
        [57.5, 0.0, 25.0, 10.0],
    ),
    (
        "block flow, rtl run after a block keeps no indent",
        r#"<div id=c style="direction:rtl;text-indent:15px;width:100px"><p style="margin:0">A</p><span></span><span id=q style="position:absolute">Q</span><p style="margin:0">A</p></div>"#,
        [75.0, 10.0, 25.0, 10.0],
    ),
    (
        "block flow, indent before a split inline element",
        r#"<div id=c style="text-indent:15px;width:100px"><span id=q style="position:absolute">Q</span><span><p style="margin:0">A</p></span></div>"#,
        [15.0, 0.0, 25.0, 10.0],
    ),
    (
        "block flow, centered run after a split inline element keeps no indent",
        r#"<div id=c style="text-indent:15px;text-align:center;width:100px"><span><p style="margin:0">A</p></span><span id=q style="position:absolute">Q</span></div>"#,
        [50.0, 10.0, 25.0, 10.0],
    ),
    (
        "block flow, hanging indent in an inline element after its block",
        r#"<div id=c style="text-indent:15px hanging;text-align:right;text-align-last:center;width:100px"><span><p style="margin:0">A</p><span id=q style="position:absolute">Q</span></span></div>"#,
        [57.5, 10.0, 10.0, 10.0],
    ),
    (
        "block flow, rtl text-align-last end in an inline element",
        r#"<div id=c style="direction:rtl;text-align-last:end;text-indent:10px;width:100px"><span><span id=q style="position:absolute">Q</span><p style="margin:0">A</p></span></div>"#,
        [-20.0, 0.0, 20.0, 10.0],
    ),
    (
        "block flow, nested inline element before a block",
        r#"<div id=c style="text-align:left;text-align-last:right;width:100px"><span><span><span id=q style="position:absolute">Q</span></span><p style="margin:0">A</p></span></div>"#,
        [100.0, 0.0, 10.0, 10.0],
    ),
    (
        "block flow, inline element after a split inline element",
        r#"<div id=c style="text-indent:15px;text-align:center;width:100px"><span><p style="margin:0">A</p></span><span><span id=q style="position:absolute">Q</span></span></div>"#,
        [50.0, 10.0, 25.0, 10.0],
    ),
    (
        "block flow, justify last line keeps the start edge",
        r#"<div id=c style="text-align:justify;text-align-last:justify;direction:rtl;width:100px"><span id=q style="position:absolute">Q</span><span></span></div>"#,
        [90.0, 0.0, 10.0, 10.0],
    ),
    (
        "block flow, text-align-last start after a block",
        r#"<div id=c style="text-align:end;text-align-last:start;width:100px"><p style="margin:0">A</p><span id=q style="position:absolute">Q</span><span><p style="margin:0">A</p></span></div>"#,
        [0.0, 10.0, 10.0, 10.0],
    ),
    (
        "block flow, two boxes before an empty inline element",
        r#"<div id=c style="text-indent:20%;text-align-last:center;width:100px"><span id=q style="position:absolute">Q</span><span id=r style="position:absolute">Q</span><span></span><p style="margin:0">A</p></div>"#,
        [60.0, 0.0, 10.0, 10.0],
    ),
    (
        "block flow, after a floated inline element",
        r#"<div id=c style="text-align:left;text-align-last:right;width:100px"><span style="float:left">x</span><span id=q style="position:absolute">Q</span><span></span><p style="margin:0">A</p></div>"#,
        [100.0, 0.0, 10.0, 10.0],
    ),
    (
        "block flow, no inline content keeps the empty line",
        r#"<div id=c style="text-indent:15px;text-align:center;width:100px"><p style="margin:0">A</p><span id=q style="position:absolute">Q</span><p style="margin:0">A</p></div>"#,
        [65.0, 10.0, 25.0, 10.0],
    ),
    (
        "block flow, a float between keeps the empty line",
        r#"<div id=c style="text-align:left;text-align-last:right;width:100px"><p style="margin:0">A</p><span id=q style="position:absolute">Q</span><div style="float:left;width:30px;height:5px"></div><p style="margin:0">A</p></div>"#,
        [0.0, 10.0, 10.0, 10.0],
    ),
    (
        "block flow, a tall float in the run sets the line in",
        r#"<div id=c style="text-indent:15px;width:100px"><span></span><div style="float:left;width:30px;height:50px"></div><span id=q style="position:absolute">Q</span><p style="margin:0">A</p></div>"#,
        [45.0, 0.0, 25.0, 10.0],
    ),
    (
        "block flow, rtl run beside an earlier float",
        r#"<div id=c style="direction:rtl;text-indent:15px;width:100px"><div style="float:left;width:30px;height:50px"></div><p style="margin:0">A</p><span></span><span id=q style="position:absolute">Q</span><p style="margin:0">A</p></div>"#,
        [75.0, 10.0, 25.0, 10.0],
    ),
    (
        "block flow, the paragraph aligns a line in an inline element, not the element",
        r#"<div id=c style="text-align:left;text-align-last:right;width:100px"><span style="text-align:center;text-align-last:left;text-indent:30px"><p style="margin:0">A</p><span id=q style="position:absolute">Q</span></span></div>"#,
        [100.0, 10.0, 40.0, 10.0],
    ),
    (
        "block flow, the paragraph direction sets the line in an inline element",
        r#"<div id=c style="direction:rtl;text-indent:15px;width:100px"><span style="direction:ltr;text-indent:30px"><span id=q style="position:absolute">Q</span><p style="margin:0">A</p></span></div>"#,
        [45.0, 0.0, 40.0, 10.0],
    ),
    (
        "block flow, the paragraph indent narrows the line in an inline element",
        r#"<div id=c style="text-indent:15px;width:100px"><span style="text-indent:30px;text-align:right"><span id=q style="position:absolute">Q</span><p style="margin:0">A</p></span></div>"#,
        [15.0, 0.0, 40.0, 10.0],
    ),
    (
        "block flow, columns align a last line after a block",
        r#"<div id=c style="columns:3;column-gap:0;text-align:left;text-align-last:right;width:90px"><p style="margin:0">B</p><span></span><span id=q style="position:absolute">Q</span><p style="margin:0">A</p></div>"#,
        [30.0, 10.0, 10.0, 10.0],
    ),
    (
        "block flow, columns narrow an indented first line",
        r#"<div id=c style="columns:3;column-gap:0;text-align:center;text-indent:6px;width:90px"><span></span><span id=q style="position:absolute">Q</span><p style="margin:0">A</p><p style="margin:0">A</p><p style="margin:0">A</p></div>"#,
        [18.0, 0.0, 16.0, 10.0],
    ),
    (
        "block flow, columns align a last line before an inline element",
        r#"<div id=c style="columns:2;column-gap:0;text-align:right;text-align-last:left;width:90px"><p style="margin:0">B</p><p style="margin:0">B</p><span id=q style="position:absolute">Q</span><span></span></div>"#,
        [45.0, 10.0, 10.0, 10.0],
    ),
    (
        "block flow, columns keep an empty line between blocks",
        r#"<div id=c style="columns:2;column-gap:0;text-align:right;width:90px"><p style="margin:0">B</p><span id=q style="position:absolute">Q</span><p style="margin:0">B</p></div>"#,
        [45.0, 10.0, 10.0, 10.0],
    ),
    (
        "block flow, a rendered legend is not a block before the first line",
        r#"<fieldset id=c style="margin:0;padding:0;border:0;text-indent:15px;width:100px"><legend>L</legend><span></span><span id=q style="position:absolute">Q</span><p style="margin:0">A</p></fieldset>"#,
        [15.0, 10.0, 25.0, 10.0],
    ),
    (
        "block flow, a rendered legend is not a block before an inline element",
        r#"<fieldset id=c style="margin:0;padding:0;border:0;text-indent:15px;width:100px"><legend>L</legend><span><span id=q style="position:absolute">Q</span><p style="margin:0">A</p></span></fieldset>"#,
        [15.0, 10.0, 25.0, 10.0],
    ),
    (
        "block flow, white space between blocks is no line content",
        r#"<div id=c style="text-align:left;text-align-last:right;width:100px"><p style="margin:0">B</p> <span id=q style="position:absolute">Q</span> <p style="margin:0">A</p></div>"#,
        [0.0, 10.0, 10.0, 10.0],
    ),
    (
        "block flow, a float before an inline element keeps its first line",
        r#"<div id=c style="text-indent:15px;width:100px"><div style="float:left;width:30px;height:50px"></div><span><span id=q style="position:absolute">Q</span><p style="margin:0">A</p></span></div>"#,
        [45.0, 0.0, 25.0, 10.0],
    ),
    (
        "block flow, a float in an inline element keeps the next one on the first line",
        r#"<div id=c style="text-indent:15px;width:100px"><span><span style="float:right;width:5px;height:50px"></span></span><span><span id=q style="position:absolute">Q</span><p style="margin:0">A</p></span></div>"#,
        [15.0, 0.0, 25.0, 10.0],
    ),
    (
        "anonymous table cell, centered alone",
        r#"<div id=c style="display:table;width:100px;text-align:center"><span id=q style="position:absolute">Q</span></div>"#,
        [50.0, 0.0, 10.0, 10.0],
    ),
    (
        "anonymous table cell, right aligned before a block",
        r#"<div id=c style="display:table;width:100px;text-align:right"><span id=q style="position:absolute">Q</span><p style="margin:0">B</p></div>"#,
        [100.0, 0.0, 10.0, 10.0],
    ),
    (
        "anonymous table cell, after a block",
        r#"<div id=c style="display:table;width:100px"><p style="margin:0">A</p><span id=q style="position:absolute">Q</span></div>"#,
        [0.0, 10.0, 10.0, 10.0],
    ),
    (
        "anonymous table cell, between blocks takes text-align, not text-align-last",
        r#"<div id=c style="display:table;width:100px;text-align:left;text-align-last:right"><p style="margin:0">A</p><span id=q style="position:absolute">Q</span><p style="margin:0">B</p></div>"#,
        [0.0, 10.0, 10.0, 10.0],
    ),
    (
        "anonymous table cell, after an element block ::before",
        r#"<style>.b::before{display:block;content:"Z"}</style><div id=c style="display:table;width:100px"><span class=b></span><span id=q style="position:absolute">Q</span></div>"#,
        [0.0, 10.0, 10.0, 10.0],
    ),
    (
        "anonymous table cell, after an empty inline element, as a last line",
        r#"<div id=c style="display:table;width:100px;text-align:left;text-align-last:right"><p style="margin:0">A</p><span></span><span id=q style="position:absolute">Q</span><p style="margin:0">B</p></div>"#,
        [100.0, 10.0, 10.0, 10.0],
    ),
    (
        "anonymous table cell, indent narrows the first anonymous line",
        r#"<div id=c style="display:table;width:100px;text-indent:15px;text-align:center"><span></span><span id=q style="position:absolute">Q</span><p style="margin:0">B</p></div>"#,
        [57.5, 0.0, 25.0, 10.0],
    ),
    (
        "anonymous table cell, after text and a block",
        r#"<div id=c style="display:table;width:100px;text-align:right">X<p style="margin:0">A</p><span id=q style="position:absolute">Q</span><p style="margin:0">B</p></div>"#,
        [100.0, 20.0, 10.0, 10.0],
    ),
    (
        "anonymous table cell, after text and a space",
        r#"<div id=c style="display:table;width:100px;text-align:center">X <span id=q style="position:absolute">Q</span></div>"#,
        [55.0, 0.0, 10.0, 10.0],
    ),
    (
        "anonymous table cell, before text that ends the line",
        r#"<div id=c style="display:table;width:100px;text-align:right"><span id=q style="position:absolute">Q</span>X<p style="margin:0">A</p></div>"#,
        [90.0, 0.0, 10.0, 10.0],
    ),
    (
        "anonymous table cell, rtl indent moves the empty line",
        r#"<div id=c style="display:table;width:100px;direction:rtl;text-indent:10px"><span id=q style="position:absolute">Q</span><p style="margin:0">A</p></div>"#,
        [70.0, 0.0, 20.0, 10.0],
    ),
    (
        "anonymous table cell, beside a float after a block",
        r#"<div id=c style="display:table;width:100px;text-align:center"><p style="margin:0;width:50px">A</p><span id=q style="position:absolute">Q</span><div style="float:left;width:30px;height:20px"></div><p style="margin:0">B</p></div>"#,
        [50.0, 10.0, 10.0, 10.0],
    ),
    (
        "anonymous table cell, after an explicit cell",
        r#"<div id=c style="display:table;width:100px;text-align:center"><span style="display:table-cell;width:40px">C</span><span id=q style="position:absolute">Q</span><p style="margin:0">A</p></div>"#,
        [70.0, 0.0, 10.0, 10.0],
    ),
    (
        "anonymous table cell, before an explicit cell",
        r#"<div id=c style="display:table;width:100px;text-align:center"><span id=q style="position:absolute">Q</span><span style="display:table-cell;width:40px">C</span></div>"#,
        [30.0, 0.0, 10.0, 10.0],
    ),
    (
        "anonymous table cell, in an inline table",
        r#"<div>X<span id=c style="display:inline-table;width:100px;text-align:right"><span id=q style="position:absolute">Q</span><p style="margin:0">A</p></span></div>"#,
        [110.0, 0.0, 10.0, 10.0],
    ),
    (
        "anonymous table cell, after a row group",
        r#"<div id=c style="display:table;width:100px;text-align:right"><span style="display:table-row-group"><span style="display:table-row"><span style="display:table-cell">R</span></span></span><span id=q style="position:absolute">Q</span><p style="margin:0">A</p></div>"#,
        [100.0, 10.0, 10.0, 10.0],
    ),
    (
        "anonymous table cell, after a caption",
        r#"<div id=c style="display:table;width:100px;text-align:center"><span style="display:table-caption">Cap</span><span id=q style="position:absolute">Q</span><p style="margin:0">A</p></div>"#,
        [50.0, 10.0, 10.0, 10.0],
    ),
    (
        "anonymous table cell, with a top inset",
        r#"<div id=c style="display:table;width:100px;text-align:center"><span id=q style="position:absolute;top:5px">Q</span><p style="margin:0">A</p></div>"#,
        [50.0, 5.0, 10.0, 10.0],
    ),
    (
        "anonymous table cell, before an element whose block ::before precedes a line",
        r#"<style>.b::before{display:block;content:"Z"}</style><div id=c style="display:table;text-align:left;width:100px"><span id=q style="position:absolute">Q</span><span class=b></span>W<br></div>"#,
        [0.0, 0.0, 10.0, 10.0],
    ),
    (
        "anonymous table cell, before an element with a block ::before and text",
        r#"<style>.b::before{display:block;content:"Z"}</style><div id=c style="display:table;text-align:left;width:100px"><span id=q style="position:absolute">Q</span><span class=b>M</span><p style="margin:0">A</p></div>"#,
        [0.0, 0.0, 10.0, 10.0],
    ),
    (
        "anonymous table cell, before an element block ::after",
        r#"<style>.b::before{display:block;content:"Z"}.a::after{display:block;content:"Y"}</style><div id=c style="display:table;text-align:left;width:100px"><span id=q style="position:absolute">Q</span><span class=a></span></div>"#,
        [0.0, 0.0, 10.0, 10.0],
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
