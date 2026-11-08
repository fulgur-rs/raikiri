//! A box in a vertical writing mode in a table cell fits its lines in the
//! height the cell gives it (CSS Writing Modes 4 §7.3.1). A cell that states a
//! length `height` makes it definite: while the row is measured that is the
//! height the cell states, and once a taller row stretches the cell past its
//! content, the height of the row. A cell without one, or with a percentage
//! or a `min-height` only, leaves its height open however far the row
//! stretches it, and the box fits the page area instead. So does inline
//! content beside a block of the cell, which sits in an anonymous block of its
//! own; a `max-height` of the cell does not lower the height.
//!
//! The expected border boxes were recorded from Chromium for the same markup in
//! an 800px by 600px viewport with no body margin. A box taller than the page
//! is split over pages here; its height is the sum of its fragments.

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

/// A case: its name, its markup with `{W}` for [`words`], and the border boxes
/// by id.
type Case = (&'static str, &'static str, &'static [(&'static str, Rect)]);

/// A hundred two-letter words: 20 to a 600px column of 10px glyphs.
fn words() -> String {
    vec!["aa"; 100].join(" ")
}

/// The border boxes of the elements of `body` with an `id`: where the first
/// fragment of each lies and how wide it is, and the heights of its fragments
/// on all pages summed.
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
    let mut boxes: Vec<(String, Rect)> = Vec::new();
    for page in result.pages() {
        let dom = page.dom();
        for fragment in page.fragments() {
            let Some(id) = dom.attr(fragment.node(), "id") else {
                continue;
            };
            let rect = fragment.rect();
            match boxes.iter_mut().find(|(box_id, _)| box_id == id) {
                Some((_, found)) => found.3 += rect.height,
                None => boxes.push((id.to_owned(), (rect.x, rect.y, rect.width, rect.height))),
            }
        }
    }
    boxes
}

/// Layout units are 1/64px; allow a little more for the 32-bit sums.
const TOLERANCE: f32 = 0.02;

fn check(name: &str, body: &str, expected: &[(&str, Rect)]) {
    let actual = border_boxes(body);
    for &(id, (x, y, width, height)) in expected {
        let found = actual
            .iter()
            .find(|(box_id, _)| box_id == id)
            .unwrap_or_else(|| panic!("{name}: no fragment for #{id}"))
            .1;
        let close = |a: f32, b: f32| (a - b).abs() <= TOLERANCE;
        assert!(
            close(found.0, x)
                && close(found.1, y)
                && close(found.2, width)
                && close(found.3, height),
            "{name}: #{id} is {found:?}, Chromium has {:?}",
            (x, y, width, height)
        );
    }
}

/// The vertical box `o` in the cell `c`. A second cell in the row that holds
/// a box of a given height stretches the row, and with it `c`.
const CASES: &[Case] = &[
    (
        "blk400",
        r#"<table style="border-spacing:0"><tr><td id=c style="padding:0"><div id=o style="writing-mode:vertical-rl">{W}</div></td><td style="padding:0"><div style="height:400px;width:10px"></div></td></tr></table>"#,
        &[
            ("c", (0.0, 0.0, 50.0, 600.0)),
            ("o", (0.0, 0.0, 50.0, 600.0)),
        ],
    ),
    (
        "ib400",
        r#"<table style="border-spacing:0"><tr><td id=c style="padding:0"><span id=o style="display:inline-block;writing-mode:vertical-rl">{W}</span></td><td style="padding:0"><div style="height:400px;width:10px"></div></td></tr></table>"#,
        &[
            ("c", (0.0, 0.0, 50.0, 602.0)),
            ("o", (0.0, 0.0, 50.0, 600.0)),
        ],
    ),
    (
        "fl400",
        r#"<table style="border-spacing:0"><tr><td id=c style="padding:0"><div id=o style="float:left;writing-mode:vertical-rl">{W}</div></td><td style="padding:0"><div style="height:400px;width:10px"></div></td></tr></table>"#,
        &[
            ("c", (0.0, 0.0, 50.0, 600.0)),
            ("o", (0.0, 0.0, 50.0, 600.0)),
        ],
    ),
    (
        "fx400",
        r#"<table style="border-spacing:0"><tr><td id=c style="padding:0"><div id=o style="display:flex;writing-mode:vertical-rl"><div>{W}</div></div></td><td style="padding:0"><div style="height:400px;width:10px"></div></td></tr></table>"#,
        &[
            ("c", (0.0, 0.0, 50.0, 600.0)),
            ("o", (0.0, 0.0, 50.0, 600.0)),
        ],
    ),
    (
        "blk602",
        r#"<table style="border-spacing:0"><tr><td id=c style="padding:0"><div id=o style="writing-mode:vertical-rl">{W}</div></td><td style="padding:0"><div style="height:602px;width:10px"></div></td></tr></table>"#,
        &[
            ("c", (0.0, 0.0, 50.0, 602.0)),
            ("o", (0.0, 1.0, 50.0, 600.0)),
        ],
    ),
    (
        "ib602",
        r#"<table style="border-spacing:0"><tr><td id=c style="padding:0"><span id=o style="display:inline-block;writing-mode:vertical-rl">{W}</span></td><td style="padding:0"><div style="height:602px;width:10px"></div></td></tr></table>"#,
        &[
            ("c", (0.0, 0.0, 50.0, 602.0)),
            ("o", (0.0, 0.0, 50.0, 600.0)),
        ],
    ),
    (
        "fl602",
        r#"<table style="border-spacing:0"><tr><td id=c style="padding:0"><div id=o style="float:left;writing-mode:vertical-rl">{W}</div></td><td style="padding:0"><div style="height:602px;width:10px"></div></td></tr></table>"#,
        &[
            ("c", (0.0, 0.0, 50.0, 602.0)),
            ("o", (0.0, 1.0, 50.0, 600.0)),
        ],
    ),
    (
        "fx602",
        r#"<table style="border-spacing:0"><tr><td id=c style="padding:0"><div id=o style="display:flex;writing-mode:vertical-rl"><div>{W}</div></div></td><td style="padding:0"><div style="height:602px;width:10px"></div></td></tr></table>"#,
        &[
            ("c", (0.0, 0.0, 50.0, 602.0)),
            ("o", (0.0, 1.0, 50.0, 600.0)),
        ],
    ),
    (
        "blk800",
        r#"<table style="border-spacing:0"><tr><td id=c style="padding:0"><div id=o style="writing-mode:vertical-rl">{W}</div></td><td style="padding:0"><div style="height:800px;width:10px"></div></td></tr></table>"#,
        &[
            ("c", (0.0, 0.0, 50.0, 800.0)),
            ("o", (0.0, 100.0, 50.0, 600.0)),
        ],
    ),
    (
        "ib800",
        r#"<table style="border-spacing:0"><tr><td id=c style="padding:0"><span id=o style="display:inline-block;writing-mode:vertical-rl">{W}</span></td><td style="padding:0"><div style="height:800px;width:10px"></div></td></tr></table>"#,
        &[
            ("c", (0.0, 0.0, 50.0, 800.0)),
            ("o", (0.0, 99.0, 50.0, 600.0)),
        ],
    ),
    (
        "fl800",
        r#"<table style="border-spacing:0"><tr><td id=c style="padding:0"><div id=o style="float:left;writing-mode:vertical-rl">{W}</div></td><td style="padding:0"><div style="height:800px;width:10px"></div></td></tr></table>"#,
        &[
            ("c", (0.0, 0.0, 50.0, 800.0)),
            ("o", (0.0, 100.0, 50.0, 600.0)),
        ],
    ),
    (
        "fx800",
        r#"<table style="border-spacing:0"><tr><td id=c style="padding:0"><div id=o style="display:flex;writing-mode:vertical-rl"><div>{W}</div></div></td><td style="padding:0"><div style="height:800px;width:10px"></div></td></tr></table>"#,
        &[
            ("c", (0.0, 0.0, 50.0, 800.0)),
            ("o", (0.0, 100.0, 50.0, 600.0)),
        ],
    ),
    (
        "blk1200",
        r#"<table style="border-spacing:0"><tr><td id=c style="padding:0"><div id=o style="writing-mode:vertical-rl">{W}</div></td><td style="padding:0"><div style="height:1200px;width:10px"></div></td></tr></table>"#,
        &[
            ("c", (0.0, 0.0, 50.0, 1200.0)),
            ("o", (0.0, 300.0, 50.0, 600.0)),
        ],
    ),
    (
        "ib1200",
        r#"<table style="border-spacing:0"><tr><td id=c style="padding:0"><span id=o style="display:inline-block;writing-mode:vertical-rl">{W}</span></td><td style="padding:0"><div style="height:1200px;width:10px"></div></td></tr></table>"#,
        &[
            ("c", (0.0, 0.0, 50.0, 1200.0)),
            ("o", (0.0, 299.0, 50.0, 600.0)),
        ],
    ),
    (
        "fl1200",
        r#"<table style="border-spacing:0"><tr><td id=c style="padding:0"><div id=o style="float:left;writing-mode:vertical-rl">{W}</div></td><td style="padding:0"><div style="height:1200px;width:10px"></div></td></tr></table>"#,
        &[
            ("c", (0.0, 0.0, 50.0, 1200.0)),
            ("o", (0.0, 300.0, 50.0, 600.0)),
        ],
    ),
    (
        "fx1200",
        r#"<table style="border-spacing:0"><tr><td id=c style="padding:0"><div id=o style="display:flex;writing-mode:vertical-rl"><div>{W}</div></div></td><td style="padding:0"><div style="height:1200px;width:10px"></div></td></tr></table>"#,
        &[
            ("c", (0.0, 0.0, 50.0, 1200.0)),
            ("o", (0.0, 300.0, 50.0, 600.0)),
        ],
    ),
    (
        "tdh",
        r#"<table style="border-spacing:0"><tr><td id=c style="padding:0;height:300px"><div id=o style="writing-mode:vertical-rl">{W}</div></td></tr></table>"#,
        &[
            ("c", (0.0, 0.0, 100.0, 300.0)),
            ("o", (0.0, 0.0, 100.0, 300.0)),
        ],
    ),
    (
        "tdh800",
        r#"<table style="border-spacing:0"><tr><td id=c style="padding:0;height:300px"><div id=o style="writing-mode:vertical-rl">{W}</div></td><td style="padding:0"><div style="height:800px;width:10px"></div></td></tr></table>"#,
        &[
            ("c", (0.0, 0.0, 100.0, 800.0)),
            ("o", (0.0, 0.0, 40.0, 800.0)),
        ],
    ),
    (
        "tblh",
        r#"<table style="border-spacing:0;height:300px"><tr><td id=c style="padding:0"><div id=o style="writing-mode:vertical-rl">{W}</div></td></tr></table>"#,
        &[
            ("c", (0.0, 0.0, 50.0, 600.0)),
            ("o", (0.0, 0.0, 50.0, 600.0)),
        ],
    ),
    (
        "trh",
        r#"<table style="border-spacing:0"><tr style="height:300px"><td id=c style="padding:0"><div id=o style="writing-mode:vertical-rl">{W}</div></td></tr></table>"#,
        &[
            ("c", (0.0, 0.0, 50.0, 600.0)),
            ("o", (0.0, 0.0, 50.0, 600.0)),
        ],
    ),
    (
        "top800",
        r#"<table style="border-spacing:0"><tr><td id=c style="padding:0;vertical-align:top"><div id=o style="writing-mode:vertical-rl">{W}</div></td><td style="padding:0"><div style="height:800px;width:10px"></div></td></tr></table>"#,
        &[
            ("c", (0.0, 0.0, 50.0, 800.0)),
            ("o", (0.0, 0.0, 50.0, 600.0)),
        ],
    ),
    (
        "h300blk",
        r#"<table style="border-spacing:0"><tr><td id=c style="padding:0;height:300px"><div id=o style="writing-mode:vertical-rl">{W}</div></td></tr></table>"#,
        &[
            ("c", (0.0, 0.0, 100.0, 300.0)),
            ("o", (0.0, 0.0, 100.0, 300.0)),
        ],
    ),
    (
        "h300ib",
        r#"<table style="border-spacing:0"><tr><td id=c style="padding:0;height:300px"><span id=o style="display:inline-block;writing-mode:vertical-rl">{W}</span></td></tr></table>"#,
        &[
            ("c", (0.0, 0.0, 100.0, 302.0)),
            ("o", (0.0, 0.0, 100.0, 300.0)),
        ],
    ),
    (
        "h300fl",
        r#"<table style="border-spacing:0"><tr><td id=c style="padding:0;height:300px"><div id=o style="float:left;writing-mode:vertical-rl">{W}</div></td></tr></table>"#,
        &[
            ("c", (0.0, 0.0, 100.0, 300.0)),
            ("o", (0.0, 0.0, 100.0, 300.0)),
        ],
    ),
    (
        "h300fx",
        r#"<table style="border-spacing:0"><tr><td id=c style="padding:0;height:300px"><div id=o style="display:flex;writing-mode:vertical-rl"><div>{W}</div></div></td></tr></table>"#,
        &[
            ("c", (0.0, 0.0, 100.0, 300.0)),
            ("o", (0.0, 0.0, 100.0, 300.0)),
        ],
    ),
    (
        "h300pad",
        r#"<table style="border-spacing:0"><tr><td id=c style="padding:10px;height:300px"><div id=o style="writing-mode:vertical-rl">{W}</div></td></tr></table>"#,
        &[
            ("c", (0.0, 0.0, 120.0, 320.0)),
            ("o", (10.0, 10.0, 100.0, 300.0)),
        ],
    ),
    (
        "h300padbb",
        r#"<table style="border-spacing:0"><tr><td id=c style="padding:10px;height:300px;box-sizing:border-box"><div id=o style="writing-mode:vertical-rl">{W}</div></td></tr></table>"#,
        &[
            ("c", (0.0, 0.0, 140.0, 300.0)),
            ("o", (10.0, 10.0, 120.0, 280.0)),
        ],
    ),
    (
        "h300top",
        r#"<table style="border-spacing:0"><tr><td id=c style="padding:0;height:300px;vertical-align:top"><div id=o style="writing-mode:vertical-rl">{W}</div></td></tr></table>"#,
        &[
            ("c", (0.0, 0.0, 100.0, 300.0)),
            ("o", (0.0, 0.0, 100.0, 300.0)),
        ],
    ),
    (
        "h300n200",
        r#"<table style="border-spacing:0"><tr><td id=c style="padding:0;height:300px"><div id=o style="writing-mode:vertical-rl">{W}</div></td><td style="padding:0"><div style="height:200px;width:10px"></div></td></tr></table>"#,
        &[
            ("c", (0.0, 0.0, 100.0, 300.0)),
            ("o", (0.0, 0.0, 100.0, 300.0)),
        ],
    ),
    (
        "h300n400",
        r#"<table style="border-spacing:0"><tr><td id=c style="padding:0;height:300px"><div id=o style="writing-mode:vertical-rl">{W}</div></td><td style="padding:0"><div style="height:400px;width:10px"></div></td></tr></table>"#,
        &[
            ("c", (0.0, 0.0, 100.0, 400.0)),
            ("o", (0.0, 0.0, 80.0, 400.0)),
        ],
    ),
    (
        "h900",
        r#"<table style="border-spacing:0"><tr><td id=c style="padding:0;height:900px"><div id=o style="writing-mode:vertical-rl">{W}</div></td></tr></table>"#,
        &[
            ("c", (0.0, 0.0, 40.0, 900.0)),
            ("o", (0.0, 0.0, 40.0, 900.0)),
        ],
    ),
    (
        "hpct",
        r#"<table style="border-spacing:0;height:300px"><tr><td id=c style="padding:0;height:50%"><div id=o style="writing-mode:vertical-rl">{W}</div></td></tr></table>"#,
        &[
            ("c", (0.0, 0.0, 50.0, 600.0)),
            ("o", (0.0, 0.0, 50.0, 600.0)),
        ],
    ),
    (
        "h300mrg",
        r#"<table style="border-spacing:0"><tr><td id=c style="padding:0;height:300px"><div id=o style="writing-mode:vertical-rl;margin:10px">{W}</div></td></tr></table>"#,
        &[
            ("c", (0.0, 0.0, 140.0, 300.0)),
            ("o", (10.0, 10.0, 120.0, 280.0)),
        ],
    ),
    (
        "trh300",
        r#"<table style="border-spacing:0"><tr style="height:300px"><td id=c style="padding:0"><div id=o style="writing-mode:vertical-rl">{W}</div></td></tr></table>"#,
        &[
            ("c", (0.0, 0.0, 50.0, 600.0)),
            ("o", (0.0, 0.0, 50.0, 600.0)),
        ],
    ),
    (
        "minh300",
        r#"<table style="border-spacing:0"><tr><td id=c style="padding:0;min-height:300px"><div id=o style="writing-mode:vertical-rl">{W}</div></td></tr></table>"#,
        &[
            ("c", (0.0, 0.0, 50.0, 600.0)),
            ("o", (0.0, 0.0, 50.0, 600.0)),
        ],
    ),
    (
        "nested",
        r#"<table style="border-spacing:0"><tr><td id=c style="padding:0;height:300px"><div id=d><div id=o style="writing-mode:vertical-rl">{W}</div></div></td></tr></table>"#,
        &[
            ("c", (0.0, 0.0, 50.0, 600.0)),
            ("d", (0.0, 0.0, 50.0, 600.0)),
            ("o", (0.0, 0.0, 50.0, 600.0)),
        ],
    ),
    (
        "span_ib",
        r#"<table style="border-spacing:0"><tr><td id=c style="padding:0;height:300px"><span><span id=o style="display:inline-block;writing-mode:vertical-rl">{W}</span></span></td></tr></table>"#,
        &[
            ("c", (0.0, 0.0, 100.0, 302.0)),
            ("o", (0.0, 0.0, 100.0, 300.0)),
        ],
    ),
    (
        "em",
        r#"<table style="border-spacing:0"><tr><td id=c style="padding:0;height:30em"><div id=o style="writing-mode:vertical-rl">{W}</div></td></tr></table>"#,
        &[
            ("c", (0.0, 0.0, 100.0, 300.0)),
            ("o", (0.0, 0.0, 100.0, 300.0)),
        ],
    ),
    (
        "rowspan",
        r#"<table style="border-spacing:0"><tr><td id=c rowspan=2 style="padding:0;height:300px"><div id=o style="writing-mode:vertical-rl">{W}</div></td><td style="padding:0">x</td></tr><tr><td style="padding:0">y</td></tr></table>"#,
        &[
            ("c", (0.0, 0.0, 100.0, 300.0)),
            ("o", (0.0, 0.0, 100.0, 300.0)),
        ],
    ),
    (
        "h50",
        r#"<table style="border-spacing:0"><tr><td id=c style="padding:0;height:50px"><div id=o style="writing-mode:vertical-rl">{W}</div></td></tr></table>"#,
        &[
            ("c", (0.0, 0.0, 500.0, 50.0)),
            ("o", (0.0, 0.0, 500.0, 50.0)),
        ],
    ),
    (
        "h300w",
        r#"<table style="border-spacing:0"><tr><td id=c style="padding:0;height:300px;width:200px"><div id=o style="writing-mode:vertical-rl">{W}</div></td></tr></table>"#,
        &[
            ("c", (0.0, 0.0, 200.0, 300.0)),
            ("o", (0.0, 0.0, 100.0, 300.0)),
        ],
    ),
    (
        "h300two",
        r#"<table style="border-spacing:0"><tr><td id=c style="padding:0;height:300px"><div id=o style="writing-mode:vertical-rl">{W}</div><div id=p style="writing-mode:vertical-rl">{W}</div></td></tr></table>"#,
        &[
            ("c", (0.0, 0.0, 100.0, 600.0)),
            ("o", (0.0, 0.0, 100.0, 300.0)),
            ("p", (0.0, 300.0, 100.0, 300.0)),
        ],
    ),
    (
        "h300spacing",
        r#"<table style="border-spacing:10px"><tr><td id=c style="padding:0;height:300px"><div id=o style="writing-mode:vertical-rl">{W}</div></td><td style="padding:0"><div style="height:400px;width:10px"></div></td></tr></table>"#,
        &[
            ("c", (10.0, 10.0, 100.0, 400.0)),
            ("o", (10.0, 10.0, 80.0, 400.0)),
        ],
    ),
    (
        "h300pct",
        r#"<table style="border-spacing:0"><tr><td id=c style="padding:0;height:300px"><div id=o style="writing-mode:vertical-rl;height:50%">{W}</div></td></tr></table>"#,
        &[
            ("c", (0.0, 0.0, 200.0, 300.0)),
            ("o", (0.0, 75.0, 200.0, 150.0)),
        ],
    ),
    (
        "h300n400bot",
        r#"<table style="border-spacing:0"><tr><td id=c style="padding:0;height:300px;vertical-align:bottom"><div id=o style="writing-mode:vertical-rl">{W}</div></td><td style="padding:0"><div style="height:400px;width:10px"></div></td></tr></table>"#,
        &[
            ("c", (0.0, 0.0, 100.0, 400.0)),
            ("o", (0.0, 0.0, 80.0, 400.0)),
        ],
    ),
    (
        "maxh",
        r#"<table style="border-spacing:0"><tr><td id=c style="padding:0;height:300px;max-height:100px"><div id=o style="writing-mode:vertical-rl">{W}</div></td></tr></table>"#,
        &[
            ("c", (0.0, 0.0, 340.0, 300.0)),
            ("o", (0.0, 0.0, 100.0, 300.0)),
        ],
    ),
    (
        "mixed",
        r#"<table style="border-spacing:0"><tr><td id=c style="padding:0;height:300px"><div id=d>x</div><span id=o style="display:inline-block;writing-mode:vertical-rl">{W}</span></td></tr></table>"#,
        &[
            ("c", (0.0, 0.0, 100.0, 612.0)),
            ("d", (0.0, 0.0, 100.0, 10.0)),
            ("o", (0.0, 10.0, 50.0, 600.0)),
        ],
    ),
    (
        "mixed2",
        r#"<table style="border-spacing:0"><tr><td id=c style="padding:0;height:300px"><div id=d>x</div><div id=o style="writing-mode:vertical-rl">{W}</div></td></tr></table>"#,
        &[
            ("c", (0.0, 0.0, 100.0, 310.0)),
            ("d", (0.0, 0.0, 100.0, 10.0)),
            ("o", (0.0, 10.0, 100.0, 300.0)),
        ],
    ),
    (
        "hcellvtbl",
        r#"<table style="border-spacing:0;writing-mode:vertical-rl"><tr><td id=c style="padding:0;height:300px;writing-mode:horizontal-tb"><div id=o style="writing-mode:vertical-rl">{W}</div></td></tr></table>"#,
        &[
            ("c", (0.0, 0.0, 100.0, 300.0)),
            ("o", (0.0, 0.0, 100.0, 300.0)),
        ],
    ),
];

#[test]
fn a_vertical_box_in_a_table_cell_fits_the_height_the_cell_gives_it() {
    let words = words();
    for &(name, body, expected) in CASES {
        check(name, &body.replace("{W}", &words), expected);
    }
}
