//! The flow-relative border properties (`border-block-start`,
//! `border-inline-end-width`, `border-block`, `border-inline-style`, ...) set
//! the border of the physical side their flow-relative side maps to under the
//! element's own writing mode and direction, and compete with the physical
//! border properties for that side in cascade order (CSS Logical Properties
//! and Values 1 §4.5).
//!
//! The expected border boxes were recorded from Chromium for the same markup
//! in an 800px by 600px viewport, with no body margin. `o` is the bordered
//! box and `c` its first child, whose position shows which sides got a
//! border. Chromium maps the inline sides of a `text-orientation: upright`
//! box with `direction: rtl` by `rtl`, while CSS Writing Modes 4 §5.1 makes
//! the used direction `ltr` and raikiri maps every flow-relative property
//! (margins and padding too) by that; those cases are left out.

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

/// A case: its name, its markup, and the border boxes by id.
type Case = (&'static str, &'static str, &'static [(&'static str, Rect)]);

/// The border boxes of the elements of `body` with an `id` on the first page.
fn border_boxes(body: &str) -> Vec<(String, Rect)> {
    let html = format!(
        "<!DOCTYPE html><style>@page {{ size: 800px 600px; margin: 0 }} \
         body {{ margin: 0; font: 10px/10px Ahem }} div {{ box-sizing: border-box }}</style>{body}"
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

/// Case names start with the writing mode: `htb` (`horizontal-tb`), `rtl`
/// (`direction: rtl`), `vrl` / `vlr` (`vertical-rl` / `vertical-lr`),
/// `vrlrtl` (`vertical-rl` with `direction: rtl`) and `slr` (`sideways-lr`).
const CASES: &[Case] = &[
    (
        "htb_bbe",
        r#"<div id=o style="border-block-end:3px solid"><div id=c>xx</div><div>x</div></div>"#,
        &[
            ("o", (0.0, 0.0, 800.0, 23.0)),
            ("c", (0.0, 0.0, 800.0, 10.0)),
        ],
    ),
    (
        "htb_bbs",
        r#"<div id=o style="border-block-start:3px solid"><div id=c>xx</div><div>x</div></div>"#,
        &[
            ("o", (0.0, 0.0, 800.0, 23.0)),
            ("c", (0.0, 3.0, 800.0, 10.0)),
        ],
    ),
    (
        "htb_bis",
        r#"<div id=o style="border-inline-start:3px solid"><div id=c>xx</div><div>x</div></div>"#,
        &[
            ("o", (0.0, 0.0, 800.0, 20.0)),
            ("c", (3.0, 0.0, 797.0, 10.0)),
        ],
    ),
    (
        "htb_bie",
        r#"<div id=o style="border-inline-end:3px solid"><div id=c>xx</div><div>x</div></div>"#,
        &[
            ("o", (0.0, 0.0, 800.0, 20.0)),
            ("c", (0.0, 0.0, 797.0, 10.0)),
        ],
    ),
    (
        "htb_bb",
        r#"<div id=o style="border-block:3px solid"><div id=c>xx</div><div>x</div></div>"#,
        &[
            ("o", (0.0, 0.0, 800.0, 26.0)),
            ("c", (0.0, 3.0, 800.0, 10.0)),
        ],
    ),
    (
        "htb_bi",
        r#"<div id=o style="border-inline:3px solid"><div id=c>xx</div><div>x</div></div>"#,
        &[
            ("o", (0.0, 0.0, 800.0, 20.0)),
            ("c", (3.0, 0.0, 794.0, 10.0)),
        ],
    ),
    (
        "htb_bbw2",
        r#"<div id=o style="border-block-style:solid;border-block-width:2px 5px"><div id=c>xx</div><div>x</div></div>"#,
        &[
            ("o", (0.0, 0.0, 800.0, 27.0)),
            ("c", (0.0, 2.0, 800.0, 10.0)),
        ],
    ),
    (
        "htb_biw2",
        r#"<div id=o style="border-inline-style:solid;border-inline-width:2px 5px"><div id=c>xx</div><div>x</div></div>"#,
        &[
            ("o", (0.0, 0.0, 800.0, 20.0)),
            ("c", (2.0, 0.0, 793.0, 10.0)),
        ],
    ),
    (
        "htb_bbs2",
        r#"<div id=o style="border-block-width:4px;border-block-style:solid none"><div id=c>xx</div><div>x</div></div>"#,
        &[
            ("o", (0.0, 0.0, 800.0, 24.0)),
            ("c", (0.0, 4.0, 800.0, 10.0)),
        ],
    ),
    (
        "htb_bisl",
        r#"<div id=o style="border-inline-start-width:4px;border-inline-start-style:solid"><div id=c>xx</div><div>x</div></div>"#,
        &[
            ("o", (0.0, 0.0, 800.0, 20.0)),
            ("c", (4.0, 0.0, 796.0, 10.0)),
        ],
    ),
    (
        "htb_bbel",
        r#"<div id=o style="border-block-end-style:solid;border-block-end-width:6px"><div id=c>xx</div><div>x</div></div>"#,
        &[
            ("o", (0.0, 0.0, 800.0, 26.0)),
            ("c", (0.0, 0.0, 800.0, 10.0)),
        ],
    ),
    (
        "htb_ord1",
        r#"<div id=o style="border-left:2px solid;border-inline-start:7px solid"><div id=c>xx</div><div>x</div></div>"#,
        &[
            ("o", (0.0, 0.0, 800.0, 20.0)),
            ("c", (7.0, 0.0, 793.0, 10.0)),
        ],
    ),
    (
        "htb_ord2",
        r#"<div id=o style="border-inline-start:7px solid;border-left:2px solid"><div id=c>xx</div><div>x</div></div>"#,
        &[
            ("o", (0.0, 0.0, 800.0, 20.0)),
            ("c", (2.0, 0.0, 798.0, 10.0)),
        ],
    ),
    (
        "htb_var",
        r#"<div id=o style="--w:4px;border-block-end:var(--w) solid"><div id=c>xx</div><div>x</div></div>"#,
        &[
            ("o", (0.0, 0.0, 800.0, 24.0)),
            ("c", (0.0, 0.0, 800.0, 10.0)),
        ],
    ),
    (
        "htb_varax",
        r#"<div id=o style="--w:4px 1px;border-inline-style:solid;border-inline-width:var(--w)"><div id=c>xx</div><div>x</div></div>"#,
        &[
            ("o", (0.0, 0.0, 800.0, 20.0)),
            ("c", (4.0, 0.0, 795.0, 10.0)),
        ],
    ),
    (
        "htb_init",
        r#"<div id=o style="border:3px solid;border-block:initial"><div id=c>xx</div><div>x</div></div>"#,
        &[
            ("o", (0.0, 0.0, 800.0, 20.0)),
            ("c", (3.0, 0.0, 794.0, 10.0)),
        ],
    ),
    (
        "htb_bbwkw",
        r#"<div id=o style="border:3px solid;border-block-start-width:initial"><div id=c>xx</div><div>x</div></div>"#,
        &[
            ("o", (0.0, 0.0, 800.0, 26.0)),
            ("c", (3.0, 3.0, 794.0, 10.0)),
        ],
    ),
    (
        "rtl_bbe",
        r#"<div id=o style="direction:rtl;border-block-end:3px solid"><div id=c>xx</div><div>x</div></div>"#,
        &[
            ("o", (0.0, 0.0, 800.0, 23.0)),
            ("c", (0.0, 0.0, 800.0, 10.0)),
        ],
    ),
    (
        "rtl_bbs",
        r#"<div id=o style="direction:rtl;border-block-start:3px solid"><div id=c>xx</div><div>x</div></div>"#,
        &[
            ("o", (0.0, 0.0, 800.0, 23.0)),
            ("c", (0.0, 3.0, 800.0, 10.0)),
        ],
    ),
    (
        "rtl_bis",
        r#"<div id=o style="direction:rtl;border-inline-start:3px solid"><div id=c>xx</div><div>x</div></div>"#,
        &[
            ("o", (0.0, 0.0, 800.0, 20.0)),
            ("c", (0.0, 0.0, 797.0, 10.0)),
        ],
    ),
    (
        "rtl_bie",
        r#"<div id=o style="direction:rtl;border-inline-end:3px solid"><div id=c>xx</div><div>x</div></div>"#,
        &[
            ("o", (0.0, 0.0, 800.0, 20.0)),
            ("c", (3.0, 0.0, 797.0, 10.0)),
        ],
    ),
    (
        "rtl_bb",
        r#"<div id=o style="direction:rtl;border-block:3px solid"><div id=c>xx</div><div>x</div></div>"#,
        &[
            ("o", (0.0, 0.0, 800.0, 26.0)),
            ("c", (0.0, 3.0, 800.0, 10.0)),
        ],
    ),
    (
        "rtl_bi",
        r#"<div id=o style="direction:rtl;border-inline:3px solid"><div id=c>xx</div><div>x</div></div>"#,
        &[
            ("o", (0.0, 0.0, 800.0, 20.0)),
            ("c", (3.0, 0.0, 794.0, 10.0)),
        ],
    ),
    (
        "rtl_bbw2",
        r#"<div id=o style="direction:rtl;border-block-style:solid;border-block-width:2px 5px"><div id=c>xx</div><div>x</div></div>"#,
        &[
            ("o", (0.0, 0.0, 800.0, 27.0)),
            ("c", (0.0, 2.0, 800.0, 10.0)),
        ],
    ),
    (
        "rtl_biw2",
        r#"<div id=o style="direction:rtl;border-inline-style:solid;border-inline-width:2px 5px"><div id=c>xx</div><div>x</div></div>"#,
        &[
            ("o", (0.0, 0.0, 800.0, 20.0)),
            ("c", (5.0, 0.0, 793.0, 10.0)),
        ],
    ),
    (
        "rtl_bbs2",
        r#"<div id=o style="direction:rtl;border-block-width:4px;border-block-style:solid none"><div id=c>xx</div><div>x</div></div>"#,
        &[
            ("o", (0.0, 0.0, 800.0, 24.0)),
            ("c", (0.0, 4.0, 800.0, 10.0)),
        ],
    ),
    (
        "rtl_bisl",
        r#"<div id=o style="direction:rtl;border-inline-start-width:4px;border-inline-start-style:solid"><div id=c>xx</div><div>x</div></div>"#,
        &[
            ("o", (0.0, 0.0, 800.0, 20.0)),
            ("c", (0.0, 0.0, 796.0, 10.0)),
        ],
    ),
    (
        "rtl_bbel",
        r#"<div id=o style="direction:rtl;border-block-end-style:solid;border-block-end-width:6px"><div id=c>xx</div><div>x</div></div>"#,
        &[
            ("o", (0.0, 0.0, 800.0, 26.0)),
            ("c", (0.0, 0.0, 800.0, 10.0)),
        ],
    ),
    (
        "rtl_ord1",
        r#"<div id=o style="direction:rtl;border-left:2px solid;border-inline-start:7px solid"><div id=c>xx</div><div>x</div></div>"#,
        &[
            ("o", (0.0, 0.0, 800.0, 20.0)),
            ("c", (2.0, 0.0, 791.0, 10.0)),
        ],
    ),
    (
        "rtl_ord2",
        r#"<div id=o style="direction:rtl;border-inline-start:7px solid;border-left:2px solid"><div id=c>xx</div><div>x</div></div>"#,
        &[
            ("o", (0.0, 0.0, 800.0, 20.0)),
            ("c", (2.0, 0.0, 791.0, 10.0)),
        ],
    ),
    (
        "rtl_var",
        r#"<div id=o style="direction:rtl;--w:4px;border-block-end:var(--w) solid"><div id=c>xx</div><div>x</div></div>"#,
        &[
            ("o", (0.0, 0.0, 800.0, 24.0)),
            ("c", (0.0, 0.0, 800.0, 10.0)),
        ],
    ),
    (
        "rtl_varax",
        r#"<div id=o style="direction:rtl;--w:4px 1px;border-inline-style:solid;border-inline-width:var(--w)"><div id=c>xx</div><div>x</div></div>"#,
        &[
            ("o", (0.0, 0.0, 800.0, 20.0)),
            ("c", (1.0, 0.0, 795.0, 10.0)),
        ],
    ),
    (
        "rtl_init",
        r#"<div id=o style="direction:rtl;border:3px solid;border-block:initial"><div id=c>xx</div><div>x</div></div>"#,
        &[
            ("o", (0.0, 0.0, 800.0, 20.0)),
            ("c", (3.0, 0.0, 794.0, 10.0)),
        ],
    ),
    (
        "rtl_bbwkw",
        r#"<div id=o style="direction:rtl;border:3px solid;border-block-start-width:initial"><div id=c>xx</div><div>x</div></div>"#,
        &[
            ("o", (0.0, 0.0, 800.0, 26.0)),
            ("c", (3.0, 3.0, 794.0, 10.0)),
        ],
    ),
    (
        "vrl_bbe",
        r#"<div id=o style="writing-mode:vertical-rl;border-block-end:3px solid"><div id=c>xx</div><div>x</div></div>"#,
        &[
            ("o", (0.0, 0.0, 23.0, 20.0)),
            ("c", (13.0, 0.0, 10.0, 20.0)),
        ],
    ),
    (
        "vrl_bbs",
        r#"<div id=o style="writing-mode:vertical-rl;border-block-start:3px solid"><div id=c>xx</div><div>x</div></div>"#,
        &[
            ("o", (0.0, 0.0, 23.0, 20.0)),
            ("c", (10.0, 0.0, 10.0, 20.0)),
        ],
    ),
    (
        "vrl_bis",
        r#"<div id=o style="writing-mode:vertical-rl;border-inline-start:3px solid"><div id=c>xx</div><div>x</div></div>"#,
        &[
            ("o", (0.0, 0.0, 20.0, 23.0)),
            ("c", (10.0, 3.0, 10.0, 20.0)),
        ],
    ),
    (
        "vrl_bie",
        r#"<div id=o style="writing-mode:vertical-rl;border-inline-end:3px solid"><div id=c>xx</div><div>x</div></div>"#,
        &[
            ("o", (0.0, 0.0, 20.0, 23.0)),
            ("c", (10.0, 0.0, 10.0, 20.0)),
        ],
    ),
    (
        "vrl_bb",
        r#"<div id=o style="writing-mode:vertical-rl;border-block:3px solid"><div id=c>xx</div><div>x</div></div>"#,
        &[
            ("o", (0.0, 0.0, 26.0, 20.0)),
            ("c", (13.0, 0.0, 10.0, 20.0)),
        ],
    ),
    (
        "vrl_bi",
        r#"<div id=o style="writing-mode:vertical-rl;border-inline:3px solid"><div id=c>xx</div><div>x</div></div>"#,
        &[
            ("o", (0.0, 0.0, 20.0, 26.0)),
            ("c", (10.0, 3.0, 10.0, 20.0)),
        ],
    ),
    (
        "vrl_bbw2",
        r#"<div id=o style="writing-mode:vertical-rl;border-block-style:solid;border-block-width:2px 5px"><div id=c>xx</div><div>x</div></div>"#,
        &[
            ("o", (0.0, 0.0, 27.0, 20.0)),
            ("c", (15.0, 0.0, 10.0, 20.0)),
        ],
    ),
    (
        "vrl_biw2",
        r#"<div id=o style="writing-mode:vertical-rl;border-inline-style:solid;border-inline-width:2px 5px"><div id=c>xx</div><div>x</div></div>"#,
        &[
            ("o", (0.0, 0.0, 20.0, 27.0)),
            ("c", (10.0, 2.0, 10.0, 20.0)),
        ],
    ),
    (
        "vrl_bbs2",
        r#"<div id=o style="writing-mode:vertical-rl;border-block-width:4px;border-block-style:solid none"><div id=c>xx</div><div>x</div></div>"#,
        &[
            ("o", (0.0, 0.0, 24.0, 20.0)),
            ("c", (10.0, 0.0, 10.0, 20.0)),
        ],
    ),
    (
        "vrl_bisl",
        r#"<div id=o style="writing-mode:vertical-rl;border-inline-start-width:4px;border-inline-start-style:solid"><div id=c>xx</div><div>x</div></div>"#,
        &[
            ("o", (0.0, 0.0, 20.0, 24.0)),
            ("c", (10.0, 4.0, 10.0, 20.0)),
        ],
    ),
    (
        "vrl_bbel",
        r#"<div id=o style="writing-mode:vertical-rl;border-block-end-style:solid;border-block-end-width:6px"><div id=c>xx</div><div>x</div></div>"#,
        &[
            ("o", (0.0, 0.0, 26.0, 20.0)),
            ("c", (16.0, 0.0, 10.0, 20.0)),
        ],
    ),
    (
        "vrl_ord1",
        r#"<div id=o style="writing-mode:vertical-rl;border-left:2px solid;border-inline-start:7px solid"><div id=c>xx</div><div>x</div></div>"#,
        &[
            ("o", (0.0, 0.0, 22.0, 27.0)),
            ("c", (12.0, 7.0, 10.0, 20.0)),
        ],
    ),
    (
        "vrl_ord2",
        r#"<div id=o style="writing-mode:vertical-rl;border-inline-start:7px solid;border-left:2px solid"><div id=c>xx</div><div>x</div></div>"#,
        &[
            ("o", (0.0, 0.0, 22.0, 27.0)),
            ("c", (12.0, 7.0, 10.0, 20.0)),
        ],
    ),
    (
        "vrl_var",
        r#"<div id=o style="writing-mode:vertical-rl;--w:4px;border-block-end:var(--w) solid"><div id=c>xx</div><div>x</div></div>"#,
        &[
            ("o", (0.0, 0.0, 24.0, 20.0)),
            ("c", (14.0, 0.0, 10.0, 20.0)),
        ],
    ),
    (
        "vrl_varax",
        r#"<div id=o style="writing-mode:vertical-rl;--w:4px 1px;border-inline-style:solid;border-inline-width:var(--w)"><div id=c>xx</div><div>x</div></div>"#,
        &[
            ("o", (0.0, 0.0, 20.0, 25.0)),
            ("c", (10.0, 4.0, 10.0, 20.0)),
        ],
    ),
    (
        "vrl_init",
        r#"<div id=o style="writing-mode:vertical-rl;border:3px solid;border-block:initial"><div id=c>xx</div><div>x</div></div>"#,
        &[
            ("o", (0.0, 0.0, 20.0, 26.0)),
            ("c", (10.0, 3.0, 10.0, 20.0)),
        ],
    ),
    (
        "vrl_bbwkw",
        r#"<div id=o style="writing-mode:vertical-rl;border:3px solid;border-block-start-width:initial"><div id=c>xx</div><div>x</div></div>"#,
        &[
            ("o", (0.0, 0.0, 26.0, 26.0)),
            ("c", (13.0, 3.0, 10.0, 20.0)),
        ],
    ),
    (
        "vlr_bbe",
        r#"<div id=o style="writing-mode:vertical-lr;border-block-end:3px solid"><div id=c>xx</div><div>x</div></div>"#,
        &[("o", (0.0, 0.0, 23.0, 20.0)), ("c", (0.0, 0.0, 10.0, 20.0))],
    ),
    (
        "vlr_bbs",
        r#"<div id=o style="writing-mode:vertical-lr;border-block-start:3px solid"><div id=c>xx</div><div>x</div></div>"#,
        &[("o", (0.0, 0.0, 23.0, 20.0)), ("c", (3.0, 0.0, 10.0, 20.0))],
    ),
    (
        "vlr_bis",
        r#"<div id=o style="writing-mode:vertical-lr;border-inline-start:3px solid"><div id=c>xx</div><div>x</div></div>"#,
        &[("o", (0.0, 0.0, 20.0, 23.0)), ("c", (0.0, 3.0, 10.0, 20.0))],
    ),
    (
        "vlr_bie",
        r#"<div id=o style="writing-mode:vertical-lr;border-inline-end:3px solid"><div id=c>xx</div><div>x</div></div>"#,
        &[("o", (0.0, 0.0, 20.0, 23.0)), ("c", (0.0, 0.0, 10.0, 20.0))],
    ),
    (
        "vlr_bb",
        r#"<div id=o style="writing-mode:vertical-lr;border-block:3px solid"><div id=c>xx</div><div>x</div></div>"#,
        &[("o", (0.0, 0.0, 26.0, 20.0)), ("c", (3.0, 0.0, 10.0, 20.0))],
    ),
    (
        "vlr_bi",
        r#"<div id=o style="writing-mode:vertical-lr;border-inline:3px solid"><div id=c>xx</div><div>x</div></div>"#,
        &[("o", (0.0, 0.0, 20.0, 26.0)), ("c", (0.0, 3.0, 10.0, 20.0))],
    ),
    (
        "vlr_bbw2",
        r#"<div id=o style="writing-mode:vertical-lr;border-block-style:solid;border-block-width:2px 5px"><div id=c>xx</div><div>x</div></div>"#,
        &[("o", (0.0, 0.0, 27.0, 20.0)), ("c", (2.0, 0.0, 10.0, 20.0))],
    ),
    (
        "vlr_biw2",
        r#"<div id=o style="writing-mode:vertical-lr;border-inline-style:solid;border-inline-width:2px 5px"><div id=c>xx</div><div>x</div></div>"#,
        &[("o", (0.0, 0.0, 20.0, 27.0)), ("c", (0.0, 2.0, 10.0, 20.0))],
    ),
    (
        "vlr_bbs2",
        r#"<div id=o style="writing-mode:vertical-lr;border-block-width:4px;border-block-style:solid none"><div id=c>xx</div><div>x</div></div>"#,
        &[("o", (0.0, 0.0, 24.0, 20.0)), ("c", (4.0, 0.0, 10.0, 20.0))],
    ),
    (
        "vlr_bisl",
        r#"<div id=o style="writing-mode:vertical-lr;border-inline-start-width:4px;border-inline-start-style:solid"><div id=c>xx</div><div>x</div></div>"#,
        &[("o", (0.0, 0.0, 20.0, 24.0)), ("c", (0.0, 4.0, 10.0, 20.0))],
    ),
    (
        "vlr_bbel",
        r#"<div id=o style="writing-mode:vertical-lr;border-block-end-style:solid;border-block-end-width:6px"><div id=c>xx</div><div>x</div></div>"#,
        &[("o", (0.0, 0.0, 26.0, 20.0)), ("c", (0.0, 0.0, 10.0, 20.0))],
    ),
    (
        "vlr_ord1",
        r#"<div id=o style="writing-mode:vertical-lr;border-left:2px solid;border-inline-start:7px solid"><div id=c>xx</div><div>x</div></div>"#,
        &[("o", (0.0, 0.0, 22.0, 27.0)), ("c", (2.0, 7.0, 10.0, 20.0))],
    ),
    (
        "vlr_ord2",
        r#"<div id=o style="writing-mode:vertical-lr;border-inline-start:7px solid;border-left:2px solid"><div id=c>xx</div><div>x</div></div>"#,
        &[("o", (0.0, 0.0, 22.0, 27.0)), ("c", (2.0, 7.0, 10.0, 20.0))],
    ),
    (
        "vlr_var",
        r#"<div id=o style="writing-mode:vertical-lr;--w:4px;border-block-end:var(--w) solid"><div id=c>xx</div><div>x</div></div>"#,
        &[("o", (0.0, 0.0, 24.0, 20.0)), ("c", (0.0, 0.0, 10.0, 20.0))],
    ),
    (
        "vlr_varax",
        r#"<div id=o style="writing-mode:vertical-lr;--w:4px 1px;border-inline-style:solid;border-inline-width:var(--w)"><div id=c>xx</div><div>x</div></div>"#,
        &[("o", (0.0, 0.0, 20.0, 25.0)), ("c", (0.0, 4.0, 10.0, 20.0))],
    ),
    (
        "vlr_init",
        r#"<div id=o style="writing-mode:vertical-lr;border:3px solid;border-block:initial"><div id=c>xx</div><div>x</div></div>"#,
        &[("o", (0.0, 0.0, 20.0, 26.0)), ("c", (0.0, 3.0, 10.0, 20.0))],
    ),
    (
        "vlr_bbwkw",
        r#"<div id=o style="writing-mode:vertical-lr;border:3px solid;border-block-start-width:initial"><div id=c>xx</div><div>x</div></div>"#,
        &[("o", (0.0, 0.0, 26.0, 26.0)), ("c", (3.0, 3.0, 10.0, 20.0))],
    ),
    (
        "vrlrtl_bbe",
        r#"<div id=o style="writing-mode:vertical-rl;direction:rtl;border-block-end:3px solid"><div id=c>xx</div><div>x</div></div>"#,
        &[
            ("o", (0.0, 0.0, 23.0, 20.0)),
            ("c", (13.0, 0.0, 10.0, 20.0)),
        ],
    ),
    (
        "vrlrtl_bbs",
        r#"<div id=o style="writing-mode:vertical-rl;direction:rtl;border-block-start:3px solid"><div id=c>xx</div><div>x</div></div>"#,
        &[
            ("o", (0.0, 0.0, 23.0, 20.0)),
            ("c", (10.0, 0.0, 10.0, 20.0)),
        ],
    ),
    (
        "vrlrtl_bis",
        r#"<div id=o style="writing-mode:vertical-rl;direction:rtl;border-inline-start:3px solid"><div id=c>xx</div><div>x</div></div>"#,
        &[
            ("o", (0.0, 0.0, 20.0, 23.0)),
            ("c", (10.0, 0.0, 10.0, 20.0)),
        ],
    ),
    (
        "vrlrtl_bie",
        r#"<div id=o style="writing-mode:vertical-rl;direction:rtl;border-inline-end:3px solid"><div id=c>xx</div><div>x</div></div>"#,
        &[
            ("o", (0.0, 0.0, 20.0, 23.0)),
            ("c", (10.0, 3.0, 10.0, 20.0)),
        ],
    ),
    (
        "vrlrtl_bb",
        r#"<div id=o style="writing-mode:vertical-rl;direction:rtl;border-block:3px solid"><div id=c>xx</div><div>x</div></div>"#,
        &[
            ("o", (0.0, 0.0, 26.0, 20.0)),
            ("c", (13.0, 0.0, 10.0, 20.0)),
        ],
    ),
    (
        "vrlrtl_bi",
        r#"<div id=o style="writing-mode:vertical-rl;direction:rtl;border-inline:3px solid"><div id=c>xx</div><div>x</div></div>"#,
        &[
            ("o", (0.0, 0.0, 20.0, 26.0)),
            ("c", (10.0, 3.0, 10.0, 20.0)),
        ],
    ),
    (
        "vrlrtl_bbw2",
        r#"<div id=o style="writing-mode:vertical-rl;direction:rtl;border-block-style:solid;border-block-width:2px 5px"><div id=c>xx</div><div>x</div></div>"#,
        &[
            ("o", (0.0, 0.0, 27.0, 20.0)),
            ("c", (15.0, 0.0, 10.0, 20.0)),
        ],
    ),
    (
        "vrlrtl_biw2",
        r#"<div id=o style="writing-mode:vertical-rl;direction:rtl;border-inline-style:solid;border-inline-width:2px 5px"><div id=c>xx</div><div>x</div></div>"#,
        &[
            ("o", (0.0, 0.0, 20.0, 27.0)),
            ("c", (10.0, 5.0, 10.0, 20.0)),
        ],
    ),
    (
        "vrlrtl_bbs2",
        r#"<div id=o style="writing-mode:vertical-rl;direction:rtl;border-block-width:4px;border-block-style:solid none"><div id=c>xx</div><div>x</div></div>"#,
        &[
            ("o", (0.0, 0.0, 24.0, 20.0)),
            ("c", (10.0, 0.0, 10.0, 20.0)),
        ],
    ),
    (
        "vrlrtl_bisl",
        r#"<div id=o style="writing-mode:vertical-rl;direction:rtl;border-inline-start-width:4px;border-inline-start-style:solid"><div id=c>xx</div><div>x</div></div>"#,
        &[
            ("o", (0.0, 0.0, 20.0, 24.0)),
            ("c", (10.0, 0.0, 10.0, 20.0)),
        ],
    ),
    (
        "vrlrtl_bbel",
        r#"<div id=o style="writing-mode:vertical-rl;direction:rtl;border-block-end-style:solid;border-block-end-width:6px"><div id=c>xx</div><div>x</div></div>"#,
        &[
            ("o", (0.0, 0.0, 26.0, 20.0)),
            ("c", (16.0, 0.0, 10.0, 20.0)),
        ],
    ),
    (
        "vrlrtl_ord1",
        r#"<div id=o style="writing-mode:vertical-rl;direction:rtl;border-left:2px solid;border-inline-start:7px solid"><div id=c>xx</div><div>x</div></div>"#,
        &[
            ("o", (0.0, 0.0, 22.0, 27.0)),
            ("c", (12.0, 0.0, 10.0, 20.0)),
        ],
    ),
    (
        "vrlrtl_ord2",
        r#"<div id=o style="writing-mode:vertical-rl;direction:rtl;border-inline-start:7px solid;border-left:2px solid"><div id=c>xx</div><div>x</div></div>"#,
        &[
            ("o", (0.0, 0.0, 22.0, 27.0)),
            ("c", (12.0, 0.0, 10.0, 20.0)),
        ],
    ),
    (
        "vrlrtl_var",
        r#"<div id=o style="writing-mode:vertical-rl;direction:rtl;--w:4px;border-block-end:var(--w) solid"><div id=c>xx</div><div>x</div></div>"#,
        &[
            ("o", (0.0, 0.0, 24.0, 20.0)),
            ("c", (14.0, 0.0, 10.0, 20.0)),
        ],
    ),
    (
        "vrlrtl_varax",
        r#"<div id=o style="writing-mode:vertical-rl;direction:rtl;--w:4px 1px;border-inline-style:solid;border-inline-width:var(--w)"><div id=c>xx</div><div>x</div></div>"#,
        &[
            ("o", (0.0, 0.0, 20.0, 25.0)),
            ("c", (10.0, 1.0, 10.0, 20.0)),
        ],
    ),
    (
        "vrlrtl_init",
        r#"<div id=o style="writing-mode:vertical-rl;direction:rtl;border:3px solid;border-block:initial"><div id=c>xx</div><div>x</div></div>"#,
        &[
            ("o", (0.0, 0.0, 20.0, 26.0)),
            ("c", (10.0, 3.0, 10.0, 20.0)),
        ],
    ),
    (
        "vrlrtl_bbwkw",
        r#"<div id=o style="writing-mode:vertical-rl;direction:rtl;border:3px solid;border-block-start-width:initial"><div id=c>xx</div><div>x</div></div>"#,
        &[
            ("o", (0.0, 0.0, 26.0, 26.0)),
            ("c", (13.0, 3.0, 10.0, 20.0)),
        ],
    ),
    (
        "slr_bbe",
        r#"<div id=o style="writing-mode:sideways-lr;border-block-end:3px solid"><div id=c>xx</div><div>x</div></div>"#,
        &[("o", (0.0, 0.0, 23.0, 20.0)), ("c", (0.0, 0.0, 10.0, 20.0))],
    ),
    (
        "slr_bbs",
        r#"<div id=o style="writing-mode:sideways-lr;border-block-start:3px solid"><div id=c>xx</div><div>x</div></div>"#,
        &[("o", (0.0, 0.0, 23.0, 20.0)), ("c", (3.0, 0.0, 10.0, 20.0))],
    ),
    (
        "slr_bis",
        r#"<div id=o style="writing-mode:sideways-lr;border-inline-start:3px solid"><div id=c>xx</div><div>x</div></div>"#,
        &[("o", (0.0, 0.0, 20.0, 23.0)), ("c", (0.0, 0.0, 10.0, 20.0))],
    ),
    (
        "slr_bie",
        r#"<div id=o style="writing-mode:sideways-lr;border-inline-end:3px solid"><div id=c>xx</div><div>x</div></div>"#,
        &[("o", (0.0, 0.0, 20.0, 23.0)), ("c", (0.0, 3.0, 10.0, 20.0))],
    ),
    (
        "slr_bb",
        r#"<div id=o style="writing-mode:sideways-lr;border-block:3px solid"><div id=c>xx</div><div>x</div></div>"#,
        &[("o", (0.0, 0.0, 26.0, 20.0)), ("c", (3.0, 0.0, 10.0, 20.0))],
    ),
    (
        "slr_bi",
        r#"<div id=o style="writing-mode:sideways-lr;border-inline:3px solid"><div id=c>xx</div><div>x</div></div>"#,
        &[("o", (0.0, 0.0, 20.0, 26.0)), ("c", (0.0, 3.0, 10.0, 20.0))],
    ),
    (
        "slr_bbw2",
        r#"<div id=o style="writing-mode:sideways-lr;border-block-style:solid;border-block-width:2px 5px"><div id=c>xx</div><div>x</div></div>"#,
        &[("o", (0.0, 0.0, 27.0, 20.0)), ("c", (2.0, 0.0, 10.0, 20.0))],
    ),
    (
        "slr_biw2",
        r#"<div id=o style="writing-mode:sideways-lr;border-inline-style:solid;border-inline-width:2px 5px"><div id=c>xx</div><div>x</div></div>"#,
        &[("o", (0.0, 0.0, 20.0, 27.0)), ("c", (0.0, 5.0, 10.0, 20.0))],
    ),
    (
        "slr_bbs2",
        r#"<div id=o style="writing-mode:sideways-lr;border-block-width:4px;border-block-style:solid none"><div id=c>xx</div><div>x</div></div>"#,
        &[("o", (0.0, 0.0, 24.0, 20.0)), ("c", (4.0, 0.0, 10.0, 20.0))],
    ),
    (
        "slr_bisl",
        r#"<div id=o style="writing-mode:sideways-lr;border-inline-start-width:4px;border-inline-start-style:solid"><div id=c>xx</div><div>x</div></div>"#,
        &[("o", (0.0, 0.0, 20.0, 24.0)), ("c", (0.0, 0.0, 10.0, 20.0))],
    ),
    (
        "slr_bbel",
        r#"<div id=o style="writing-mode:sideways-lr;border-block-end-style:solid;border-block-end-width:6px"><div id=c>xx</div><div>x</div></div>"#,
        &[("o", (0.0, 0.0, 26.0, 20.0)), ("c", (0.0, 0.0, 10.0, 20.0))],
    ),
    (
        "slr_ord1",
        r#"<div id=o style="writing-mode:sideways-lr;border-left:2px solid;border-inline-start:7px solid"><div id=c>xx</div><div>x</div></div>"#,
        &[("o", (0.0, 0.0, 22.0, 27.0)), ("c", (2.0, 0.0, 10.0, 20.0))],
    ),
    (
        "slr_ord2",
        r#"<div id=o style="writing-mode:sideways-lr;border-inline-start:7px solid;border-left:2px solid"><div id=c>xx</div><div>x</div></div>"#,
        &[("o", (0.0, 0.0, 22.0, 27.0)), ("c", (2.0, 0.0, 10.0, 20.0))],
    ),
    (
        "slr_var",
        r#"<div id=o style="writing-mode:sideways-lr;--w:4px;border-block-end:var(--w) solid"><div id=c>xx</div><div>x</div></div>"#,
        &[("o", (0.0, 0.0, 24.0, 20.0)), ("c", (0.0, 0.0, 10.0, 20.0))],
    ),
    (
        "slr_varax",
        r#"<div id=o style="writing-mode:sideways-lr;--w:4px 1px;border-inline-style:solid;border-inline-width:var(--w)"><div id=c>xx</div><div>x</div></div>"#,
        &[("o", (0.0, 0.0, 20.0, 25.0)), ("c", (0.0, 1.0, 10.0, 20.0))],
    ),
    (
        "slr_init",
        r#"<div id=o style="writing-mode:sideways-lr;border:3px solid;border-block:initial"><div id=c>xx</div><div>x</div></div>"#,
        &[("o", (0.0, 0.0, 20.0, 26.0)), ("c", (0.0, 3.0, 10.0, 20.0))],
    ),
    (
        "slr_bbwkw",
        r#"<div id=o style="writing-mode:sideways-lr;border:3px solid;border-block-start-width:initial"><div id=c>xx</div><div>x</div></div>"#,
        &[("o", (0.0, 0.0, 26.0, 26.0)), ("c", (3.0, 3.0, 10.0, 20.0))],
    ),
    (
        "inherit_axis",
        r#"<div style="border:4px solid"><div id=o style="border-inline:inherit"><div id=c>x</div></div></div>"#,
        &[
            ("o", (4.0, 4.0, 792.0, 10.0)),
            ("c", (8.0, 4.0, 784.0, 10.0)),
        ],
    ),
    (
        "inherit_vrl",
        r#"<div style="border-left:4px solid;border-top:2px solid"><div id=o style="writing-mode:vertical-rl;border-block-end:inherit;border-inline-start:inherit"><div id=c>x</div></div></div>"#,
        &[("o", (4.0, 2.0, 14.0, 12.0)), ("c", (8.0, 4.0, 10.0, 10.0))],
    ),
    (
        "htb_varside",
        r#"<div id=o style="--b:4px solid;border-inline-end:var(--b)"><div id=c>xx</div><div>x</div></div>"#,
        &[
            ("o", (0.0, 0.0, 800.0, 20.0)),
            ("c", (0.0, 0.0, 796.0, 10.0)),
        ],
    ),
    (
        "htb_varbb",
        r#"<div id=o style="--b:2px solid;border-block:var(--b)"><div id=c>xx</div><div>x</div></div>"#,
        &[
            ("o", (0.0, 0.0, 800.0, 24.0)),
            ("c", (0.0, 2.0, 800.0, 10.0)),
        ],
    ),
    (
        "htb_varbbs",
        r#"<div id=o style="--s:solid;border-block-width:3px 6px;border-block-style:var(--s)"><div id=c>xx</div><div>x</div></div>"#,
        &[
            ("o", (0.0, 0.0, 800.0, 29.0)),
            ("c", (0.0, 3.0, 800.0, 10.0)),
        ],
    ),
    (
        "htb_inhw",
        r#"<div style="border:4px solid;border-top-width:1px;border-left-width:2px"><div id=o style="border-block-start-width:inherit;border-block-start-style:solid"><div id=c>xx</div></div></div>"#,
        &[
            ("o", (2.0, 1.0, 794.0, 11.0)),
            ("c", (2.0, 2.0, 794.0, 10.0)),
        ],
    ),
    (
        "htb_revlayer",
        r#"<style>@layer a{#o{border-inline-start:5px solid}} @layer b{#o{border-inline-start:revert-layer}}</style><div id=o style=""><div id=c>xx</div></div>"#,
        &[
            ("o", (0.0, 0.0, 800.0, 10.0)),
            ("c", (5.0, 0.0, 795.0, 10.0)),
        ],
    ),
    (
        "htb_unset",
        r#"<div id=o style="border:5px solid;border-inline:unset"><div id=c>xx</div><div>x</div></div>"#,
        &[
            ("o", (0.0, 0.0, 800.0, 30.0)),
            ("c", (0.0, 5.0, 800.0, 10.0)),
        ],
    ),
    (
        "htb_bbstyle",
        r#"<div id=o style="border-block-width:4px;border-block-style:none solid"><div id=c>xx</div><div>x</div></div>"#,
        &[
            ("o", (0.0, 0.0, 800.0, 24.0)),
            ("c", (0.0, 0.0, 800.0, 10.0)),
        ],
    ),
    (
        "htb_bicolor",
        r#"<div id=o style="border-inline:4px solid;border-inline-color:red"><div id=c>xx</div><div>x</div></div>"#,
        &[
            ("o", (0.0, 0.0, 800.0, 20.0)),
            ("c", (4.0, 0.0, 792.0, 10.0)),
        ],
    ),
    (
        "vrl_varside",
        r#"<div id=o style="writing-mode:vertical-rl;--b:4px solid;border-inline-end:var(--b)"><div id=c>xx</div><div>x</div></div>"#,
        &[
            ("o", (0.0, 0.0, 20.0, 24.0)),
            ("c", (10.0, 0.0, 10.0, 20.0)),
        ],
    ),
    (
        "vrl_varbb",
        r#"<div id=o style="writing-mode:vertical-rl;--b:2px solid;border-block:var(--b)"><div id=c>xx</div><div>x</div></div>"#,
        &[
            ("o", (0.0, 0.0, 24.0, 20.0)),
            ("c", (12.0, 0.0, 10.0, 20.0)),
        ],
    ),
    (
        "vrl_varbbs",
        r#"<div id=o style="writing-mode:vertical-rl;--s:solid;border-block-width:3px 6px;border-block-style:var(--s)"><div id=c>xx</div><div>x</div></div>"#,
        &[
            ("o", (0.0, 0.0, 29.0, 20.0)),
            ("c", (16.0, 0.0, 10.0, 20.0)),
        ],
    ),
    (
        "vrl_inhw",
        r#"<div style="border:4px solid;border-top-width:1px;border-left-width:2px"><div id=o style="writing-mode:vertical-rl;border-block-start-width:inherit;border-block-start-style:solid"><div id=c>xx</div></div></div>"#,
        &[("o", (2.0, 1.0, 14.0, 20.0)), ("c", (2.0, 1.0, 10.0, 20.0))],
    ),
    (
        "vrl_revlayer",
        r#"<style>@layer a{#o{border-inline-start:5px solid}} @layer b{#o{border-inline-start:revert-layer}}</style><div id=o style="writing-mode:vertical-rl;"><div id=c>xx</div></div>"#,
        &[("o", (0.0, 0.0, 10.0, 25.0)), ("c", (0.0, 5.0, 10.0, 20.0))],
    ),
    (
        "vrl_unset",
        r#"<div id=o style="writing-mode:vertical-rl;border:5px solid;border-inline:unset"><div id=c>xx</div><div>x</div></div>"#,
        &[
            ("o", (0.0, 0.0, 30.0, 20.0)),
            ("c", (15.0, 0.0, 10.0, 20.0)),
        ],
    ),
    (
        "vrl_bbstyle",
        r#"<div id=o style="writing-mode:vertical-rl;border-block-width:4px;border-block-style:none solid"><div id=c>xx</div><div>x</div></div>"#,
        &[
            ("o", (0.0, 0.0, 24.0, 20.0)),
            ("c", (14.0, 0.0, 10.0, 20.0)),
        ],
    ),
    (
        "vrl_bicolor",
        r#"<div id=o style="writing-mode:vertical-rl;border-inline:4px solid;border-inline-color:red"><div id=c>xx</div><div>x</div></div>"#,
        &[
            ("o", (0.0, 0.0, 20.0, 28.0)),
            ("c", (10.0, 4.0, 10.0, 20.0)),
        ],
    ),
    (
        "vlr_varside",
        r#"<div id=o style="writing-mode:vertical-lr;--b:4px solid;border-inline-end:var(--b)"><div id=c>xx</div><div>x</div></div>"#,
        &[("o", (0.0, 0.0, 20.0, 24.0)), ("c", (0.0, 0.0, 10.0, 20.0))],
    ),
    (
        "vlr_varbb",
        r#"<div id=o style="writing-mode:vertical-lr;--b:2px solid;border-block:var(--b)"><div id=c>xx</div><div>x</div></div>"#,
        &[("o", (0.0, 0.0, 24.0, 20.0)), ("c", (2.0, 0.0, 10.0, 20.0))],
    ),
    (
        "vlr_varbbs",
        r#"<div id=o style="writing-mode:vertical-lr;--s:solid;border-block-width:3px 6px;border-block-style:var(--s)"><div id=c>xx</div><div>x</div></div>"#,
        &[("o", (0.0, 0.0, 29.0, 20.0)), ("c", (3.0, 0.0, 10.0, 20.0))],
    ),
    (
        "vlr_inhw",
        r#"<div style="border:4px solid;border-top-width:1px;border-left-width:2px"><div id=o style="writing-mode:vertical-lr;border-block-start-width:inherit;border-block-start-style:solid"><div id=c>xx</div></div></div>"#,
        &[("o", (2.0, 1.0, 12.0, 20.0)), ("c", (4.0, 1.0, 10.0, 20.0))],
    ),
    (
        "vlr_revlayer",
        r#"<style>@layer a{#o{border-inline-start:5px solid}} @layer b{#o{border-inline-start:revert-layer}}</style><div id=o style="writing-mode:vertical-lr;"><div id=c>xx</div></div>"#,
        &[("o", (0.0, 0.0, 10.0, 25.0)), ("c", (0.0, 5.0, 10.0, 20.0))],
    ),
    (
        "vlr_unset",
        r#"<div id=o style="writing-mode:vertical-lr;border:5px solid;border-inline:unset"><div id=c>xx</div><div>x</div></div>"#,
        &[("o", (0.0, 0.0, 30.0, 20.0)), ("c", (5.0, 0.0, 10.0, 20.0))],
    ),
    (
        "vlr_bbstyle",
        r#"<div id=o style="writing-mode:vertical-lr;border-block-width:4px;border-block-style:none solid"><div id=c>xx</div><div>x</div></div>"#,
        &[("o", (0.0, 0.0, 24.0, 20.0)), ("c", (0.0, 0.0, 10.0, 20.0))],
    ),
    (
        "vlr_bicolor",
        r#"<div id=o style="writing-mode:vertical-lr;border-inline:4px solid;border-inline-color:red"><div id=c>xx</div><div>x</div></div>"#,
        &[("o", (0.0, 0.0, 20.0, 28.0)), ("c", (0.0, 4.0, 10.0, 20.0))],
    ),
    (
        "rtl_varside",
        r#"<div id=o style="direction:rtl;--b:4px solid;border-inline-end:var(--b)"><div id=c>xx</div><div>x</div></div>"#,
        &[
            ("o", (0.0, 0.0, 800.0, 20.0)),
            ("c", (4.0, 0.0, 796.0, 10.0)),
        ],
    ),
    (
        "rtl_varbb",
        r#"<div id=o style="direction:rtl;--b:2px solid;border-block:var(--b)"><div id=c>xx</div><div>x</div></div>"#,
        &[
            ("o", (0.0, 0.0, 800.0, 24.0)),
            ("c", (0.0, 2.0, 800.0, 10.0)),
        ],
    ),
    (
        "rtl_varbbs",
        r#"<div id=o style="direction:rtl;--s:solid;border-block-width:3px 6px;border-block-style:var(--s)"><div id=c>xx</div><div>x</div></div>"#,
        &[
            ("o", (0.0, 0.0, 800.0, 29.0)),
            ("c", (0.0, 3.0, 800.0, 10.0)),
        ],
    ),
    (
        "rtl_inhw",
        r#"<div style="border:4px solid;border-top-width:1px;border-left-width:2px"><div id=o style="direction:rtl;border-block-start-width:inherit;border-block-start-style:solid"><div id=c>xx</div></div></div>"#,
        &[
            ("o", (2.0, 1.0, 794.0, 11.0)),
            ("c", (2.0, 2.0, 794.0, 10.0)),
        ],
    ),
    (
        "rtl_revlayer",
        r#"<style>@layer a{#o{border-inline-start:5px solid}} @layer b{#o{border-inline-start:revert-layer}}</style><div id=o style="direction:rtl;"><div id=c>xx</div></div>"#,
        &[
            ("o", (0.0, 0.0, 800.0, 10.0)),
            ("c", (0.0, 0.0, 795.0, 10.0)),
        ],
    ),
    (
        "rtl_unset",
        r#"<div id=o style="direction:rtl;border:5px solid;border-inline:unset"><div id=c>xx</div><div>x</div></div>"#,
        &[
            ("o", (0.0, 0.0, 800.0, 30.0)),
            ("c", (0.0, 5.0, 800.0, 10.0)),
        ],
    ),
    (
        "rtl_bbstyle",
        r#"<div id=o style="direction:rtl;border-block-width:4px;border-block-style:none solid"><div id=c>xx</div><div>x</div></div>"#,
        &[
            ("o", (0.0, 0.0, 800.0, 24.0)),
            ("c", (0.0, 0.0, 800.0, 10.0)),
        ],
    ),
    (
        "rtl_bicolor",
        r#"<div id=o style="direction:rtl;border-inline:4px solid;border-inline-color:red"><div id=c>xx</div><div>x</div></div>"#,
        &[
            ("o", (0.0, 0.0, 800.0, 20.0)),
            ("c", (4.0, 0.0, 792.0, 10.0)),
        ],
    ),
    (
        "rev_phys",
        r#"<style>@layer a{#o{border-left:5px solid}} @layer b{#o{border-inline-start:revert-layer}}</style><div id=o style="writing-mode:vertical-rl"><div id=c>xx</div></div>"#,
        &[("o", (0.0, 0.0, 15.0, 20.0)), ("c", (5.0, 0.0, 10.0, 20.0))],
    ),
    (
        "rev_log",
        r#"<style>@layer a{#o{border-block-end:5px solid}} @layer b{#o{border-left:revert-layer}}</style><div id=o style="writing-mode:vertical-rl;border-left:1px solid"><div id=c>xx</div></div>"#,
        &[("o", (0.0, 0.0, 11.0, 20.0)), ("c", (1.0, 0.0, 10.0, 20.0))],
    ),
];

#[test]
fn flow_relative_borders_match_chromium() {
    let mut failures = Vec::new();
    for &(name, body, expected) in CASES {
        let boxes = border_boxes(body);
        for &(id, want) in expected {
            let got = boxes.iter().find(|(found, _)| found == id).map(|(_, r)| *r);
            if got != Some(want) {
                failures.push(format!("{name} #{id}: expected {want:?}, got {got:?}"));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
